// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `dlist`: run a cartridge for N frames, then decode the display list ANTIC
//! will run next frame, with the registers that shape it.
//!
//!   dlist <cart> [--frames N] [--map mirror|one|two|linear|bob]

use antic_core::cart::{Cart, Mapping};
use antic_core::machine::Machine;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut cart_path = None;
    let mut frames = 600u64;
    let mut map = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--frames" => {
                i += 1;
                frames = args[i].parse().unwrap();
            }
            "--map" => {
                i += 1;
                map = Some(match args[i].as_str() {
                    "mirror" => Mapping::Mirror,
                    "one" => Mapping::OneChip16,
                    "two" => Mapping::TwoChip16,
                    "linear" => Mapping::Linear32,
                    _ => Mapping::BountyBob40,
                });
            }
            a => cart_path = Some(a.to_string()),
        }
        i += 1;
    }
    let rom = std::fs::read(cart_path.expect("usage: dlist <cart>")).unwrap();
    let bios = std::fs::read(root.join("bios/[BIOS] Atari 5200 (USA).a52")).unwrap();
    let cart = match map {
        Some(m) => Cart::new(rom, m).unwrap(),
        None => Cart::with_default_mapping(rom).unwrap(),
    };
    let mut m = Machine::new(cart, Some(&bios));
    for _ in 0..frames {
        m.run_frame();
    }
    // Stop at the start of the display region so DLIST is the frame's start.
    while m.bus.antic.vline != 8 {
        m.step();
    }
    let a = &m.bus.antic;
    let g = &m.bus.gtia;
    println!(
        "DMACTL={:02X} CHACTL={:02X} CHBASE={:02X} HSCROL={:X} VSCROL={:X} NMIEN={:02X} PRIOR={:02X} GRACTL={:02X}",
        a.dmactl, a.chactl, a.chbase, a.hscrol, a.vscrol, a.nmien, g.regs[0x1b], g.regs[0x1d]
    );
    let peek = |x: u16| m.bus.peek(x).unwrap_or(0xff);
    let mut pc = a.dlist;
    let mut line = 8u32;
    for _ in 0..200 {
        let ir = peek(pc);
        let mode = ir & 0x0f;
        let mut text = format!("{pc:04X}: {ir:02X}  line {line:3}  ");
        let next = (pc & 0xfc00) | (pc.wrapping_add(1) & 0x3ff);
        let mut after = next;
        let height: u32 = match mode {
            0 => ((ir >> 4) & 7) as u32 + 1,
            1 => 1,
            2 | 4 | 6 | 8 => 8,
            3 => 10,
            5 | 7 => 16,
            9 | 0xa => 4,
            0xb | 0xd => 2,
            _ => 1,
        };
        if mode == 0 {
            text += &format!("blank x{height}");
        } else if mode == 1 {
            let t = u16::from_le_bytes([peek(next), peek(next.wrapping_add(1))]);
            text += &format!("{} {t:04X}", if ir & 0x40 != 0 { "JVB" } else { "JMP" });
        } else {
            text += &format!("mode {mode:X}");
            if ir & 0x40 != 0 {
                let t = u16::from_le_bytes([peek(next), peek(next.wrapping_add(1))]);
                text += &format!(" LMS {t:04X}");
                after = next.wrapping_add(2);
            }
        }
        for (bit, name) in [(0x80, " DLI"), (0x20, " VS"), (0x10, " HS")] {
            if ir & bit != 0 && mode >= 2 || (bit == 0x80 && ir & 0x80 != 0 && mode < 2) {
                text += name;
            }
        }
        println!("{text}");
        if mode == 1 {
            break;
        }
        line += height;
        pc = after;
        if line >= 248 {
            println!("(reached line 248)");
            break;
        }
    }
}
