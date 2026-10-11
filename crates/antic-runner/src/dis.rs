// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `dis`: disassemble a range of an image loaded at a base address.
//!
//!   dis <file> <base-hex> <from-hex> [count]
//!
//! The BIOS is `dis bios.a52 F800 FFFC` to see the reset vector region.
//! Output stays local: a BIOS listing is not ours to commit.

use antic_core::disasm::disasm;

fn hex(s: &str) -> u16 {
    u16::from_str_radix(s.trim_start_matches('$'), 16).expect("hex address")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: dis <file> <base-hex> <from-hex> [count]");
        std::process::exit(2);
    }
    let image = std::fs::read(&args[0]).expect("read image");
    let base = hex(&args[1]);
    let mut pc = hex(&args[2]);
    let count: usize = args.get(3).map(|s| s.parse().expect("count")).unwrap_or(40);
    let peek = |a: u16| {
        let off = a.wrapping_sub(base) as usize;
        image.get(off).copied().unwrap_or(0)
    };
    for _ in 0..count {
        let (text, len) = disasm(pc, peek);
        let bytes: Vec<String> = (0..len).map(|i| format!("{:02X}", peek(pc.wrapping_add(i)))).collect();
        println!("{pc:04X}  {:<9} {text}", bytes.join(" "));
        pc = pc.wrapping_add(len);
    }
}
