// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! Cartridge mapping for the 32K window at $4000-$BFFF.
//!
//! The mapping is NOT a property of the file size alone, and that is half of
//! the hashing problem: a 16K image can be one 16K chip or two 8K chips, which
//! place the same bytes at different addresses. The corpus census
//! (docs/notes/HASHING-CORPUS-CENSUS.md) found 16K images written out as 32K
//! in the two-chip layout `[lo, lo, hi, hi]`, which [`Mapping::Linear32`]
//! then runs correctly. Which mapping a given 16K title needs is measured by
//! booting it both ways, not guessed here.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mapping {
    /// 4K or 8K: the image repeats through the whole window.
    Mirror,
    /// 16K, one chip: the image at $8000-$BFFF, mirrored at $4000-$7FFF.
    OneChip16,
    /// 16K, two 8K chips: first half at $4000-$7FFF (twice), second half
    /// at $8000-$BFFF (twice).
    TwoChip16,
    /// 32K straight through $4000-$BFFF.
    Linear32,
    /// 40K Bounty Bob Strikes Back: two 4K windows at $4000 and $5000, each
    /// switching among four banks on an access to $xFF6-$xFF9, and the last
    /// 8K fixed at $8000, mirrored at $A000.
    BountyBob40,
}

impl Mapping {
    /// The mappings a file of this size could need. More than one means the
    /// size does not decide it.
    pub fn candidates(len: usize) -> &'static [Mapping] {
        match len {
            4096 | 8192 => &[Mapping::Mirror],
            16384 => &[Mapping::TwoChip16, Mapping::OneChip16],
            32768 => &[Mapping::Linear32],
            40960 => &[Mapping::BountyBob40],
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CartError {
    /// No mapping handles an image of this size.
    UnsupportedSize(usize),
    /// The mapping does not fit an image of this size.
    WrongSize(Mapping, usize),
}

#[derive(Clone, Debug)]
pub struct Cart {
    rom: Vec<u8>,
    pub mapping: Mapping,
    bank: [u8; 2],
}

impl Cart {
    pub fn new(rom: Vec<u8>, mapping: Mapping) -> Result<Self, CartError> {
        let ok = match mapping {
            Mapping::Mirror => rom.len() == 4096 || rom.len() == 8192,
            Mapping::OneChip16 | Mapping::TwoChip16 => rom.len() == 16384,
            Mapping::Linear32 => rom.len() == 32768,
            Mapping::BountyBob40 => rom.len() == 40960,
        };
        if !ok {
            return Err(CartError::WrongSize(mapping, rom.len()));
        }
        Ok(Cart { rom, mapping, bank: [0; 2] })
    }

    /// Pick the first candidate mapping for this size.
    pub fn with_default_mapping(rom: Vec<u8>) -> Result<Self, CartError> {
        let m = *Mapping::candidates(rom.len())
            .first()
            .ok_or(CartError::UnsupportedSize(rom.len()))?;
        Cart::new(rom, m)
    }

    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    pub fn bank(&self) -> [u8; 2] {
        self.bank
    }

    pub fn set_bank(&mut self, bank: [u8; 2]) {
        self.bank = [bank[0] & 3, bank[1] & 3];
    }

    /// File offset for a window address, or None for open bus.
    fn offset(&self, addr: u16) -> Option<usize> {
        let a = addr.wrapping_sub(0x4000) as usize; // 0..0x7FFF in the window
        Some(match self.mapping {
            Mapping::Mirror => a % self.rom.len(),
            Mapping::OneChip16 => a & 0x3fff,
            Mapping::TwoChip16 => (a & 0x1fff) | if a >= 0x4000 { 0x2000 } else { 0 },
            Mapping::Linear32 => a,
            Mapping::BountyBob40 => match addr {
                0x4000..=0x4fff => self.bank[0] as usize * 0x1000 + (addr as usize - 0x4000),
                0x5000..=0x5fff => 0x4000 + self.bank[1] as usize * 0x1000 + (addr as usize - 0x5000),
                0x8000..=0xbfff => 0x8000 + (addr as usize & 0x1fff),
                _ => return None,
            },
        })
    }

    /// Side-effect-free read, for DMA, disassembly and probes.
    pub fn peek(&self, addr: u16) -> Option<u8> {
        self.offset(addr).map(|o| self.rom[o])
    }

    /// Bus access, read or write: Bounty Bob switches banks on either.
    pub fn access(&mut self, addr: u16) {
        if self.mapping == Mapping::BountyBob40 {
            match addr {
                0x4ff6..=0x4ff9 => self.bank[0] = (addr - 0x4ff6) as u8,
                0x5ff6..=0x5ff9 => self.bank[1] = (addr - 0x5ff6) as u8,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(len: usize) -> Vec<u8> {
        // Each byte names its own 4K block, so a wrong mapping is visible.
        (0..len).map(|i| (i / 0x1000) as u8).collect()
    }

    #[test]
    fn two_chip_16k_puts_each_half_in_its_own_16k_and_mirrors_it() {
        let c = Cart::new(image(16384), Mapping::TwoChip16).unwrap();
        let blocks: Vec<u8> = (0..8).map(|k| c.peek(0x4000 + k * 0x1000).unwrap()).collect();
        assert_eq!(blocks, vec![0, 1, 0, 1, 2, 3, 2, 3]);
    }

    #[test]
    fn two_chip_16k_runs_like_its_32k_expansion() {
        // The corpus stores some 16K titles as [lo, lo, hi, hi]. Mapped
        // linearly, that must read exactly like the 16K under TwoChip16.
        let rom = image(16384);
        let mut big = Vec::new();
        big.extend_from_slice(&rom[..0x2000]);
        big.extend_from_slice(&rom[..0x2000]);
        big.extend_from_slice(&rom[0x2000..]);
        big.extend_from_slice(&rom[0x2000..]);
        let two = Cart::new(rom, Mapping::TwoChip16).unwrap();
        let lin = Cart::new(big, Mapping::Linear32).unwrap();
        for a in (0x4000u32..0xc000).step_by(0x100) {
            assert_eq!(two.peek(a as u16), lin.peek(a as u16), "${a:04X}");
        }
    }

    #[test]
    fn one_chip_16k_mirrors_the_whole_image() {
        let c = Cart::new(image(16384), Mapping::OneChip16).unwrap();
        let blocks: Vec<u8> = (0..8).map(|k| c.peek(0x4000 + k * 0x1000).unwrap()).collect();
        assert_eq!(blocks, vec![0, 1, 2, 3, 0, 1, 2, 3]);
    }

    #[test]
    fn bounty_bob_switches_each_window_on_access() {
        let mut c = Cart::new(image(40960), Mapping::BountyBob40).unwrap();
        assert_eq!(c.peek(0x4000), Some(0));
        assert_eq!(c.peek(0x5000), Some(4));
        c.access(0x4ff8);
        c.access(0x5ff9);
        assert_eq!(c.peek(0x4000), Some(2));
        assert_eq!(c.peek(0x5000), Some(7));
        assert_eq!(c.peek(0x8000), Some(8));
        assert_eq!(c.peek(0xa000), Some(8));
        assert_eq!(c.peek(0x6000), None);
    }
}
