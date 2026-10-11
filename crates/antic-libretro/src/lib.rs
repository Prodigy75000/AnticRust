// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! The libretro C ABI for AnticRust. Not implemented yet.
//!
//! On most cores this layer is plumbing. On this one it is half the product,
//! because the reason the project exists is that RetroAchievements cannot
//! support the Atari 5200 on any existing core, and both failures live exactly
//! here rather than in the emulation.
//!
//! # What this layer owes, specifically
//!
//! **`SET_MEMORY_MAPS` and `SYSTEM_RAM`.** Omitting them leaves RA hanging on
//! "waiting for core memory map". The `a5200` core does not expose memory at
//! all, which is the whole of RA's complaint about it.
//!
//! **A stable hashing method.** RA has to identify a cartridge the same way
//! every time. What the hash covers, and what it must EXCLUDE, is a decision
//! this core owes rather than inherits: dumps in the wild may or may not carry
//! a header, and a hash that includes one identifies the dumper rather than the
//! game.
//!
//! **No way to load content.** The frontend hands the core a cartridge and that
//! is the only path. `atari800` is unusable for achievements because its
//! internal menu can open a file the frontend never saw, so the bytes RA hashed
//! and the bytes executing can differ. This core should have no file IO for
//! content at all; see `antic_core::ContentSource`, which is a single-variant
//! enum guarded by a test for that reason.
//!
//! **Pixel format, declared explicitly at init.** A core that assumes the
//! frontend default gets colours subtly wrong on one platform and right on
//! another, which is miserable to chase from a screenshot.
//!
//! **Input descriptors that say what the controls ARE.** RA's note on
//! `atari800` includes not being able to work out the controls. The 5200 stick
//! is analog and does not self-centre, so this is not a formality.
//!
//! Deliberately empty for now. No stub exports: an `extern "C"` returning a
//! plausible value is worse than a missing symbol, because the frontend loads
//! the core, gets an answer, and fails somewhere far away.
