// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `antic`: what the core makes of a cartridge.
//!
//!   antic [cart]
//!
//! With no cartridge, prints the machine constants. With one, prints its
//! size, CRC-32, the mapping the core picks for it, and the size of the
//! canonical image the RetroAchievements hash is taken over.

use antic_core::cart::{crc32, Cart};
use antic_core::{hash, BIOS_SIZE, CPU_HZ, RAM, SAVE_STATE_VERSION};

fn main() {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("--help" | "-h")) {
        println!("usage: antic [cart]");
        return;
    }

    println!("AnticRust {}", env!("CARGO_PKG_VERSION"));
    println!("  6502C SALLY   {CPU_HZ:>9} Hz");
    println!("  RAM           {RAM:>9} bytes");
    println!("  BIOS window   {BIOS_SIZE:>9} bytes (HLE, no dump needed)");
    println!("  save state    v{SAVE_STATE_VERSION}");

    let Some(path) = arg else { return };
    let rom = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    });
    println!();
    println!("{path}");
    println!("  size          {:>9} bytes", rom.len());
    println!("  CRC-32        {:>9}", format!("{:08x}", crc32(&rom)));
    println!("  canonical     {:>9} bytes", hash::canonical(&rom).len());
    match Cart::identified(rom) {
        Ok(cart) => println!("  mapping       {:?}", cart.mapping),
        Err(e) => println!("  mapping       none ({e:?})"),
    }
}
