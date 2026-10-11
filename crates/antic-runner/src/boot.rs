// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `boot`: run a cartridge on the real BIOS and say what happened.
//!
//!   boot <cart> [--bios <file>|--hle] [--frames N] [--map ...] [--script]
//!
//! --dump ADDR LEN prints memory at the end of the run.
//!
//! --script drives the census's generic input (Start, fire, top, keypad 1,
//! stick sweep, a 240-frame cycle from frame 300). If the CPU jams, the run
//! stops there and the report shows the jam address and how it got there.
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
    let mut dump: Option<(u16, usize)> = None;
    let mut script = false;
    let mut script_from = 300u64;
    let mut hle = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bios" => { i += 1; bios_path = args[i].clone(); }
            "--frames" => { i += 1; frames = args[i].parse().expect("frames"); }
            "--script" => script = true,
            "--dump" => {
                let a = u16::from_str_radix(args[i + 1].trim_start_matches('$'), 16).expect("hex");
                let n = args[i + 2].parse().expect("len");
                dump = Some((a, n));
                i += 2;
            }
            "--script-from" => { i += 1; script = true; script_from = args[i].parse().unwrap(); }
            "--hle" => hle = true,
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
    let mut m = Machine::new(cart, if hle { None } else { Some(&bios) });
    let watch = Rc::new(RefCell::new(Watch::default()));
    m.bus.probe = Some(Box::new(Shared(watch.clone())));
    let mut trail: std::collections::VecDeque<u16> = std::collections::VecDeque::new();
    for f in 0..frames {
        if f + 1 == frames {
            watch.borrow_mut().count = true;
        }
        if script {
            census_script(&mut m, f, script_from);
        }
        let end = m.bus.antic.frame;
        while m.bus.antic.frame == end && !m.cpu.halted {
            trail.push_back(m.cpu.pc);
            if trail.len() > 80 {
                trail.pop_front();
            }
            m.step();
        }
        if m.cpu.halted {
            println!("JAMMED in frame {f} at ${:04X}; the last instructions:", m.cpu.pc.wrapping_sub(1));
            for pc in &trail {
                let (text, _) = disasm(*pc, |a| m.bus.peek(a).unwrap_or(0xff));
                println!("    ${pc:04X}  {text}");
            }
            break;
        }
    }
    if let Some((a, n)) = dump {
        let bytes: Vec<String> = (0..n).map(|k| format!("{:02X}", m.bus.peek(a.wrapping_add(k as u16)).unwrap_or(0xff))).collect();
        println!("dump ${a:04X}: {}", bytes.join(" "));
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

/// The census's generic input, kept in step with census.rs.
fn census_script(m: &mut Machine, f: u64, from: u64) {
    use antic_core::pokey::Key;
    if f < from {
        return;
    }
    let phase = (f - from) % 240;
    let c = &mut m.bus.pokey.controllers[0];
    c.key = match phase {
        60..=67 => Some(Key::Start),
        200..=207 => Some(Key::Digit(1)),
        _ => None,
    };
    c.top = (170..178).contains(&phase);
    let t = f as f64;
    c.pot = [
        (114.0 + 110.0 * (t / 40.0).sin()).round().clamp(1.0, 228.0) as u8,
        (114.0 + 110.0 * (t / 57.0).cos()).round().clamp(1.0, 228.0) as u8,
    ];
    m.bus.gtia.trigger[0] = (120..130).contains(&phase) || (150..160).contains(&phase);
}
