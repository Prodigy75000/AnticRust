// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! The canonical image a RetroAchievements hash should cover.
//!
//! The same cartridge circulates at several sizes (docs/notes/
//! HASHING-CORPUS-CENSUS.md): an 8K image repeated to 16K or 32K, and a
//! two-chip 16K image written out as 32K in the order `[lo, lo, hi, hi]`. A
//! hash over the file's bytes calls each of those a different game, so a set
//! approved against one dump would not load another. This reduces every such
//! file to one image, by rules that never merge two images which run
//! differently:
//!
//! 1. **A `CART` header is dropped.** It is atari800's container: the magic
//!    `CART`, a big-endian type, a checksum, then the image. No file in our
//!    corpus has one, but the format exists, and a header identifies the
//!    dumper, not the game.
//! 2. **An image that is its first half twice is halved,** down to 4K. In
//!    the 32K window a repeated image reads the same at every address, so
//!    halving it changes nothing a game can observe.
//! 3. **A 32K image of 8K blocks `[A, A, B, B]` becomes `A ++ B`,** the
//!    two-chip 16K cartridge it is a written-out copy of. Mapped linearly,
//!    `[A, A, B, B]` puts A at $4000 and $6000 and B at $8000 and $A000,
//!    which is exactly the two-chip mapping of `A ++ B`.
//!
//! A 40K Bounty Bob image and any size not listed are left as they are.
//! `hashing_preserves_what_runs` in the runner checks rules 2 and 3 against
//! the machine: original and canonical image, frame for frame.

use crate::cart::Mapping;

/// The `CART` container header, when present: (header length, image length).
fn cart_header(rom: &[u8]) -> Option<usize> {
    if rom.len() > 16 && &rom[..4] == b"CART" && is_image_size(rom.len() - 16) {
        Some(16)
    } else {
        None
    }
}

fn is_image_size(n: usize) -> bool {
    matches!(n, 4096 | 8192 | 16384 | 32768 | 40960)
}

/// The bytes the hash covers.
pub fn canonical(rom: &[u8]) -> Vec<u8> {
    reduce(rom).0
}

/// The canonical image and, when the reduction itself says how the original
/// was laid out, the mapping that runs the canonical image the way the
/// original ran: a 32K image that halved to 16K was a one-chip 16K mirrored
/// through the window, and a `[A, A, B, B]` was a two-chip one. None means
/// the reduction is silent and the usual identification applies.
pub fn reduce(rom: &[u8]) -> (Vec<u8>, Option<Mapping>) {
    let mut img: &[u8] = rom;
    if let Some(h) = cart_header(img) {
        img = &img[h..];
    }
    let start = img.len();
    // Rule 2: halve while the image is its first half repeated.
    while img.len() > 4096
        && img.len().is_power_of_two()
        && img[..img.len() / 2] == img[img.len() / 2..]
    {
        img = &img[..img.len() / 2];
    }
    // Rule 3: a written-out two-chip 16K.
    if img.len() == 32768 {
        let b = |k: usize| &img[k * 0x2000..(k + 1) * 0x2000];
        if b(0) == b(1) && b(2) == b(3) {
            let mut out = Vec::with_capacity(16384);
            out.extend_from_slice(b(0));
            out.extend_from_slice(b(2));
            return (out, Some(Mapping::TwoChip16));
        }
    }
    let implied = if img.len() == 16384 && start == 32768 {
        Some(Mapping::OneChip16)
    } else {
        None
    };
    (img.to_vec(), implied)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block filled with one value, so blocks are told apart by content.
    fn block(v: u8, n: usize) -> Vec<u8> {
        vec![v; n]
    }

    #[test]
    fn a_repeated_8k_reduces_to_itself_from_16k_and_32k() {
        let mut x = block(1, 4096);
        x.extend(block(2, 4096));
        let x16 = [x.clone(), x.clone()].concat();
        let x32 = [x16.clone(), x16.clone()].concat();
        assert_eq!(canonical(&x16), x);
        assert_eq!(canonical(&x32), x);
        assert_eq!(
            canonical(&x),
            x,
            "an 8K image with distinct halves stays 8K"
        );
    }

    #[test]
    fn a_written_out_two_chip_16k_reduces_to_its_two_chips() {
        let (a, b) = (block(3, 0x2000), block(4, 0x2000));
        let wide = [a.clone(), a.clone(), b.clone(), b.clone()].concat();
        assert_eq!(canonical(&wide), [a, b].concat());
    }

    #[test]
    fn a_real_32k_and_a_bounty_bob_are_left_alone() {
        let real = [
            block(1, 0x2000),
            block(2, 0x2000),
            block(3, 0x2000),
            block(4, 0x2000),
        ]
        .concat();
        assert_eq!(canonical(&real), real);
        // [A, B, A, B] is not two-chip, but it is its first half twice, so it
        // halves to [A, B]: a 16K image mirrored to 32K.
        let abab = [
            block(1, 0x2000),
            block(2, 0x2000),
            block(1, 0x2000),
            block(2, 0x2000),
        ]
        .concat();
        assert_eq!(
            canonical(&abab),
            [block(1, 0x2000), block(2, 0x2000)].concat()
        );
        let bob = (0..40960).map(|i| (i / 4096) as u8).collect::<Vec<_>>();
        assert_eq!(canonical(&bob), bob);
    }

    #[test]
    fn each_reduction_names_the_layout_it_undid() {
        let (a, b) = (block(3, 0x2000), block(4, 0x2000));
        let x = [a.clone(), b.clone()].concat();
        assert_eq!(
            reduce(&[a.clone(), a.clone(), b.clone(), b.clone()].concat()).1,
            Some(Mapping::TwoChip16)
        );
        assert_eq!(
            reduce(&[x.clone(), x.clone()].concat()).1,
            Some(Mapping::OneChip16)
        );
        assert_eq!(reduce(&x).1, None, "an untouched 16K says nothing");
    }

    #[test]
    fn a_cart_header_is_dropped_and_a_lookalike_is_not() {
        let img = [block(5, 4096), block(6, 4096)].concat();
        let mut headed = b"CART".to_vec();
        headed.extend_from_slice(&[0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0]);
        headed.extend_from_slice(&img);
        assert_eq!(canonical(&headed), img);
        // An image that merely starts with "CART" but has no room for a
        // header keeps its bytes.
        let mut plain = img.clone();
        plain[..4].copy_from_slice(b"CART");
        assert_eq!(canonical(&plain), plain);
    }
}
