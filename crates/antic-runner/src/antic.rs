// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `antic`: the first dev harness. Boot something and step it.
//!
//! Nothing to boot yet. This exists so the workspace has a binary that builds
//! and runs from day one: the first thing a new core needs is a way to say
//! "here is the machine, here is what it did".

use antic_core::{BiosMode, ContentSource, BIOS_SIZE, CPU_HZ, RAM, SAVE_STATE_VERSION};

fn main() {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("antic: AnticRust dev harness");
        println!();
        println!("usage: antic [--help]");
        println!();
        println!("Nothing is implemented yet. Two objectives: an HLE BIOS so no");
        println!("firmware is needed, and a core RetroAchievements can approve");
        println!("a set against, which no existing Atari 5200 core can.");
        return;
    }

    println!("AnticRust {}", env!("CARGO_PKG_VERSION"));
    println!("save-state format : v{SAVE_STATE_VERSION}");
    println!("BIOS mode         : {:?}", BiosMode::default());
    println!("content source    : {:?} (the only one there is)", ContentSource::default());
    println!();
    println!("  6502C SALLY   {:>9} Hz", CPU_HZ);
    println!("  RAM           {:>9} bytes", RAM);
    println!("  BIOS window   {:>9} bytes", BIOS_SIZE);
    println!();
    println!("No CPU, no ANTIC, no GTIA, no POKEY, no HLE. Nothing to run.");
}
