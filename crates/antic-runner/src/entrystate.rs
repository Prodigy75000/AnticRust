// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `entrystate`: the machine state the real BIOS hands a cartridge, measured
//! at the first instruction fetched from the cartridge window.
//!
//!   entrystate [--dir <folder>] [--hle]
//!
//! With --hle, each title is booted on BOTH the real BIOS and the HLE, and
//! the state at entry is compared: every difference in zero page, the RAM
//! vectors, the CPU registers and the chip registers is listed.
//!
//! For every full-boot title it records zero page, $0200-$021F, the CPU
//! registers and the stateful chip registers last written, then prints the
//! bytes that are the SAME for every title (the BIOS's own post-boot state)
//! and the ones that differ (which depend on the cartridge). This is what an
//! HLE boot has to synthesise.

use antic_core::cart::{Cart, Mapping};
use antic_core::machine::{Access, AccessKind, Machine, Probe};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Default)]
struct Watch {
    entered: bool,
    regs: BTreeMap<u16, u8>,
}

struct Shared(Rc<RefCell<Watch>>);
impl Probe for Shared {
    fn access(&mut self, a: &Access) {
        let mut w = self.0.borrow_mut();
        if w.entered {
            return;
        }
        match a.kind {
            AccessKind::Fetch if (0x4000..0xc000).contains(&a.addr) => w.entered = true,
            AccessKind::Write if a.addr >= 0xc000 && a.addr < 0xf000 => {
                let canon = match a.addr {
                    0xc000..=0xcfff => 0xc000 | (a.addr & 0x1f),
                    0xd400..=0xd4ff => 0xd400 | (a.addr & 0x0f),
                    _ => 0xe800 | (a.addr & 0x0f),
                };
                w.regs.insert(canon, a.value);
            }
            _ => {}
        }
    }
}

struct Snap {
    title: String,
    ram: Vec<u8>,
    regs: BTreeMap<u16, u8>,
    cpu: (u8, u8, u8, u8, u8),
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dir = root.join("dumps/Atari 5200 ROM Collection/Commerical Games (1982-1984)");
    let hle = args.iter().any(|a| a == "--hle");
    if let Some(k) = args.iter().position(|a| a == "--dir") {
        dir = PathBuf::from(&args[k + 1]);
    }
    let bios = std::fs::read(root.join("bios/[BIOS] Atari 5200 (USA).a52")).unwrap();
    let mut snaps = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    if hle {
        compare(&files, &bios);
        return;
    }
    for p in files {
        let rom = std::fs::read(&p).unwrap();
        if rom.len() < 3 || rom[rom.len() - 3] == 0xff {
            continue; // skip-boot: nothing is set up
        }
        let Some(&m0) = Mapping::candidates(rom.len()).first() else { continue };
        let mut m = Machine::new(Cart::new(rom, m0).unwrap(), Some(&bios));
        let w = Rc::new(RefCell::new(Watch::default()));
        m.bus.probe = Some(Box::new(Shared(w.clone())));
        // Step until the cart's first instruction is about to run.
        while m.cpu.pc < 0x4000 || m.cpu.pc >= 0xc000 {
            m.step();
        }
        let regs = w.borrow().regs.clone();
        let mut ram = m.bus.ram[0x000..0x100].to_vec();
        ram.extend_from_slice(&m.bus.ram[0x200..0x220]);
        snaps.push(Snap {
            title: p.file_name().unwrap().to_string_lossy().into_owned(),
            ram,
            regs,
            cpu: (m.cpu.a, m.cpu.x, m.cpu.y, m.cpu.sp, m.cpu.p),
        });
    }
    println!("{} full-boot titles", snaps.len());
    let addr = |i: usize| if i < 0x100 { i as u16 } else { 0x200 + (i - 0x100) as u16 };
    let mut same = Vec::new();
    let mut differ = Vec::new();
    for i in 0..snaps[0].ram.len() {
        let v = snaps[0].ram[i];
        if snaps.iter().all(|s| s.ram[i] == v) {
            if v != 0 {
                same.push(format!("{:04X}={v:02X}", addr(i)));
            }
        } else {
            differ.push(format!("{:04X}", addr(i)));
        }
    }
    println!("RAM equal across titles and nonzero: {}", same.join(" "));
    println!("RAM that differs by title: {}", differ.join(" "));
    let mut rsame = Vec::new();
    for (r, v) in &snaps[0].regs {
        if snaps.iter().all(|s| s.regs.get(r) == Some(v)) && *v != 0 {
            rsame.push(format!("{r:04X}={v:02X}"));
        }
    }
    println!("chip registers last written, equal and nonzero: {}", rsame.join(" "));
    let c = snaps[0].cpu;
    let cpu_same = snaps.iter().all(|s| s.cpu == c);
    println!("CPU a={:02X} x={:02X} y={:02X} sp={:02X} p={:02X} (same for all: {cpu_same})", c.0, c.1, c.2, c.3, c.4);
    for s in snaps.iter().take(3) {
        let d: Vec<String> = differ.iter().map(|a| {
            let a = u16::from_str_radix(a, 16).unwrap();
            let i = if a < 0x100 { a as usize } else { 0x100 + (a - 0x200) as usize };
            format!("{:02X}", s.ram[i])
        }).collect();
        println!("  {:40} differing bytes: {}", &s.title[..s.title.len().min(40)], d.join(" "));
    }
}

/// Boot to the cart's first instruction; return (zp + $0200-$021F, chip
/// registers last written, cpu registers).
fn entry(rom: &[u8], bios: Option<&[u8]>) -> (Vec<u8>, BTreeMap<u16, u8>, (u8, u8, u8, u8, u8)) {
    let m0 = Mapping::candidates(rom.len())[0];
    let mut m = Machine::new(Cart::new(rom.to_vec(), m0).unwrap(), bios);
    let w = Rc::new(RefCell::new(Watch::default()));
    m.bus.probe = Some(Box::new(Shared(w.clone())));
    while m.cpu.pc < 0x4000 || m.cpu.pc >= 0xc000 {
        m.step();
    }
    let mut ram = m.bus.ram[0x000..0x100].to_vec();
    ram.extend_from_slice(&m.bus.ram[0x200..0x220]);
    let regs = w.borrow().regs.clone();
    (ram, regs, (m.cpu.a, m.cpu.x, m.cpu.y, m.cpu.sp, m.cpu.p))
}

fn compare(files: &[PathBuf], bios: &[u8]) {
    let mut identical = 0;
    let mut n = 0;
    // Strobes: writing them is an event, not state.
    let strobe = |r: u16| matches!(r, 0xd40a | 0xd40f | 0xc01e | 0xe809 | 0xe80a | 0xe80b | 0xe80d);
    for p in files {
        let rom = std::fs::read(p).unwrap();
        if rom.len() < 3 || Mapping::candidates(rom.len()).is_empty() {
            continue;
        }
        n += 1;
        let (ra, ga, ca) = entry(&rom, Some(bios));
        let (rb, gb, cb) = entry(&rom, None);
        let mut diffs = Vec::new();
        for i in 0..ra.len() {
            if ra[i] != rb[i] {
                let a = if i < 0x100 { i } else { 0x200 + i - 0x100 };
                diffs.push(format!("{a:04X}:{:02X}/{:02X}", ra[i], rb[i]));
            }
        }
        let keys: std::collections::BTreeSet<u16> = ga.keys().chain(gb.keys()).copied().collect();
        for r in keys {
            if strobe(r) {
                continue;
            }
            let (x, y) = (ga.get(&r).copied().unwrap_or(0), gb.get(&r).copied().unwrap_or(0));
            if x != y {
                diffs.push(format!("reg {r:04X}:{x:02X}/{y:02X}"));
            }
        }
        if ca != cb {
            diffs.push(format!("cpu {ca:02X?}/{cb:02X?}"));
        }
        let title = p.file_name().unwrap().to_string_lossy();
        if diffs.is_empty() {
            identical += 1;
        } else {
            println!("{title}: {}", diffs.join(" "));
        }
    }
    println!("{identical} of {n} titles enter with identical zero page, vectors, chip registers and CPU (real/HLE)");
}
