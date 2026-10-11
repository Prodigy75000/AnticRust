// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `boot`: run a cartridge on the real BIOS and say what happened.
//!
//!   boot <cart> [--bios <file>] [--frames N] [--map mirror|one|two|linear|bob]
//!
//! Reports the first instruction fetched from the cartridge window and when,
//! then what the CPU spent the last frame executing (a PC histogram with
//! disassembly), and the ANTIC state. When every counter says fine, ask what
//! it is executing.

use antic_core::cart::{Cart, Mapping};
use antic_core::disasm::disasm;
use antic_core::machine::{Access, AccessKind, Machine, Probe};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Default)]
struct Watch {
    first_cart_fetch: Option<(u16, u64)>,
    hist: HashMap<u16, u64>,
    count: bool,
}

struct Shared(Rc<RefCell<Watch>>);
impl Probe for Shared {
    fn access(&mut self, a: &Access) {
        if a.kind != AccessKind::Fetch {
            return;
        }
        let mut w = self.0.borrow_mut();
        if w.first_cart_fetch.is_none() && (0x4000..0xc000).contains(&a.addr) {
            w.first_cart_fetch = Some((a.addr, a.cycle));
        }
        if w.count {
            *w.hist.entry(a.addr).or_default() += 1;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cart_path = None;
    let mut bios_path = String::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../bios/[BIOS] Atari 5200 (USA).a52"));
    let mut frames = 300u64;
    let mut map = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bios" => { i += 1; bios_path = args[i].clone(); }
            "--frames" => { i += 1; frames = args[i].parse().expect("frames"); }
            "--map" => {
                i += 1;
                map = Some(match args[i].as_str() {
                    "mirror" => Mapping::Mirror,
                    "one" => Mapping::OneChip16,
                    "two" => Mapping::TwoChip16,
                    "linear" => Mapping::Linear32,
                    "bob" => Mapping::BountyBob40,
                    m => panic!("unknown mapping {m}"),
                });
            }
            a => cart_path = Some(a.to_string()),
        }
        i += 1;
    }
    let Some(cart_path) = cart_path else {
        eprintln!("usage: boot <cart> [--bios <file>] [--frames N] [--map mirror|one|two|linear|bob]");
        std::process::exit(2);
    };
    let rom = std::fs::read(&cart_path).expect("read cart");
    let bios = std::fs::read(&bios_path).expect("read bios");
    let cart = match map {
        Some(m) => Cart::new(rom, m).expect("mapping"),
        None => Cart::with_default_mapping(rom).expect("mapping"),
    };
    println!("cart    : {cart_path} ({:?})", cart.mapping);
    let mut m = Machine::new(cart, Some(&bios));
    let watch = Rc::new(RefCell::new(Watch::default()));
    m.bus.probe = Some(Box::new(Shared(watch.clone())));
    for f in 0..frames {
        if f + 1 == frames {
            watch.borrow_mut().count = true;
        }
        m.run_frame();
    }
    let w = watch.borrow();
    match w.first_cart_fetch {
        Some((pc, cyc)) => println!(
            "entered : ${pc:04X} at cycle {cyc} (frame {:.2})",
            cyc as f64 / 29868.0
        ),
        None => println!("entered : NEVER (no fetch from $4000-$BFFF in {frames} frames)"),
    }
    let a = &m.bus.antic;
    println!(
        "antic   : dmactl=${:02X} dlist=${:04X} nmien=${:02X} chbase=${:02X} frame={}",
        a.dmactl, a.dlist, a.nmien, a.chbase, a.frame
    );
    let mut top: Vec<_> = w.hist.iter().collect();
    top.sort_by(|x, y| y.1.cmp(x.1));
    println!("last frame, most-fetched instructions:");
    for (pc, n) in top.iter().take(12) {
        let (text, _) = disasm(**pc, |a| m.bus.peek(a).unwrap_or(0xff));
        println!("  {n:>6}  ${pc:04X}  {text}");
    }
}
