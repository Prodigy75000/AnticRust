// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! AnticRust: the machine.
//!
//! A clean-room Atari 5200 core. Nothing is implemented yet; this file holds
//! the constraints so they are in front of whoever writes the first module
//! rather than in a chat log.
//!
//! # Two objectives, and the second is why the project exists
//!
//! **No BIOS.** The HLE boots a cartridge with no firmware dump present, the
//! same way ColecoRust and IntelliRust do.
//!
//! **Be the core a RetroAchievements set can be approved against.** RA lists
//! this machine as unsupported, needing "hashing method and memory map". The
//! two existing libretro cores fail it in different ways: one does not expose
//! memory at all, and the other allows loading arbitrary files from its own
//! internal menu, which bypasses hashing because the bytes running are then not
//! the bytes the frontend hashed.
//!
//! Both failures have obvious opposites, and they are constraints on this
//! crate rather than aspirations:
//!
//! - publish `SET_MEMORY_MAPS` and `SYSTEM_RAM`, which is already a standing
//!   rule for every in-house core and is the whole point on this one,
//! - **never load content from inside the core.** A core with no way to open a
//!   file cannot be made to run something the frontend did not hash. If a
//!   convenience file picker is ever proposed here, this is the reason to
//!   refuse it,
//! - no BIOS configuration surface, which the HLE objective gives for free.
//!
//! # Binding contracts inherited from the other in-house cores
//!
//! - **Save states are byte-identical across architectures.** Fixed size,
//!   little-endian, versioned. Same bytes on x86-64 and arm64, because netplay
//!   between a PC and a phone rests on it. Rules out serialising anything
//!   host-shaped: pointers, `usize`, hash iteration order, enum discriminants a
//!   compiler may renumber.
//! - **A real BIOS is an ORACLE, never a requirement.**
//!
//! # Two things about this machine that will catch a 6502 author out
//!
//! **ANTIC is not a video chip you poke.** It is a coprocessor that fetches and
//! executes its own display list out of RAM, steals cycles from the CPU to do
//! it, and drives GTIA. Timing is not a detail layered on afterwards; the CPU
//! and ANTIC share a bus and the contention is observable.
//!
//! **The joysticks are analog and do NOT self-centre.** They are read as pot
//! values through POKEY, not as switch closures. A digital stick mapped to the
//! extremes will feel wrong and will also be wrong, and "centre" is a value the
//! hardware never reliably returns on its own.

pub mod antic;
pub mod cart;
pub mod cpu;
pub mod disasm;
pub mod font;
pub mod gtia;
pub mod hle;
pub mod machine;
pub mod pokey;
pub mod save;

/// 6502C "SALLY" clock, 1.79 MHz NTSC.
pub const CPU_HZ: u32 = 1_789_773;

/// Work RAM, in bytes.
pub const RAM: usize = 16 * 1024;

/// The BIOS is 2 KB.
///
/// Under HLE nothing is loaded for it, but the ADDRESSES still matter: titles
/// call documented entry points, and that is the surface the HLE presents.
pub const BIOS_SIZE: usize = 2 * 1024;

/// Save-state format version. Bump on ANY layout change.
pub const SAVE_STATE_VERSION: u32 = 0;

/// How the machine gets its BIOS behaviour.
///
/// The default is [`BiosMode::Hle`] on purpose. A firmware dump is a
/// development oracle, and treating it as the normal path is how a project ends
/// up shipping something that needs one, which is the exact complaint RA makes
/// about the existing cores.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BiosMode {
    /// Reimplemented entry points. No dump present, and none needed.
    #[default]
    Hle,
    /// A real dump, for differential testing against the HLE.
    Real,
}

/// Where a cartridge may come from.
///
/// There is deliberately only one variant. It exists as a type so that adding a
/// second one is a visible decision in a diff rather than a quiet convenience,
/// because the second variant is exactly what makes a core unusable for
/// achievements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ContentSource {
    /// Handed to the core by the frontend, which has already hashed it.
    #[default]
    Frontend,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hle_is_the_default_path_rather_than_the_fallback() {
        // If this flips, the project has quietly become one that needs a
        // firmware file, which is the first thing it exists to avoid.
        assert_eq!(BiosMode::default(), BiosMode::Hle);
    }

    #[test]
    fn the_core_can_only_be_handed_content_by_the_frontend() {
        // The guard against the atari800 failure mode. RA cannot approve a core
        // that can run bytes the frontend never hashed, and the only way to be
        // sure of that is to have no other way in. If someone adds a variant
        // here, this test fails and they have to argue for it in the open.
        assert_eq!(ContentSource::default(), ContentSource::Frontend);
        // Exhaustiveness: one arm today, and a compile error the day it is not.
        match ContentSource::default() {
            ContentSource::Frontend => {}
        }
    }

    #[test]
    fn the_machine_constants_are_the_real_ones() {
        // 1.79 MHz, not a round 2 MHz; 16 KB, not 64; the BIOS is 2 KB, which is
        // a quarter of the ColecoVision's and is the number people assume is
        // wrong.
        assert_eq!(CPU_HZ, 1_789_773);
        assert_eq!(RAM, 16_384);
        assert_eq!(BIOS_SIZE, 2_048);
    }
}
