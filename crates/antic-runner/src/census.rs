// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `census`: what does each cartridge actually use of the real BIOS?
//!
//!   census [--dir <folder>] [--frames N] [--out <file>] [--input]
//!
//! Every title runs on the real BIOS with no input, and a probe sees every
//! bus cycle. Per title it records:
//!
//! - **boot**: which path the BIOS took ($BFFD = $FF enters the cart at once),
//!   the cycles of SETUP (reset until the BIOS starts its logo wait loop) and
//!   of WAIT (the visible logo, until the first fetch from the cart);
//! - **calls**: control transfers from cart or RAM code into $F800-$FFFF that
//!   are not interrupt entries, by target;
//! - **interrupts**: which hardware vector was taken, and where the BIOS
//!   dispatcher handed control back out (the RAM vectors in use);
//! - **data**: non-fetch reads of $F800-$FFFF by cart or RAM code, and ANTIC
//!   DMA of that region (the font at $F800 is the usual reason);
//! - **ram**: RAM bytes the cart reads whose last writer was BIOS code, split
//!   into written during boot and written at run time (by a BIOS handler).
//!   The stack page is counted apart: a BIOS handler pushing registers that
//!   the cart's own handler pulls is a calling convention, not shared state;
//! - **regs**: stateful hardware registers the BIOS wrote during boot that the
//!   cart never wrote, with the value, so the cart runs on the BIOS's setting.
//!
//! These are CANDIDATE dependencies: "read before the cart wrote it" includes
//! reads whose value never mattered. Re-running the same titles with the HLE
//! and diffing is what settles them.
//!
//! 16K images run under both mappings, since the size does not decide it.
//! Nothing from the BIOS image itself is printed beyond addresses.

use antic_core::cart::{Cart, Mapping};
use antic_core::machine::{Access, AccessKind, Machine, Probe};
use antic_core::pokey::Key;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const BIOS_LO: u16 = 0xf800;
/// The BIOS logo wait: CPX $02 / BNE * until the frame counter wraps.
const WAIT_LOOP: u16 = 0xfe8e;
const CYCLES_PER_FRAME: f64 = 29868.0;

fn in_bios(a: u16) -> bool {
    a >= BIOS_LO
}
fn in_cart(a: u16) -> bool {
    (0x4000..0xc000).contains(&a)
}

/// Fold a hardware register address to its canonical name, keeping only
/// registers that HOLD state. Strobes (WSYNC, NMIRES, HITCLR, STIMER, SKRES,
/// POTGO, SEROUT) and unassigned addresses are dropped: the BIOS clear loop
/// writes all of them, and inheriting a strobe means nothing.
fn canon_reg(a: u16) -> Option<(u16, &'static str)> {
    match a {
        0xc000..=0xcfff => {
            let r = a & 0x1f;
            (r != 0x1e).then_some((0xc000 | r, "GTIA"))
        }
        0xd400..=0xd4ff => {
            let r = a & 0x0f;
            matches!(r, 0x0..=0x5 | 0x7 | 0x9 | 0xe).then_some((0xd400 | r, "ANTIC"))
        }
        0xe800..=0xefff => {
            let r = a & 0x0f;
            matches!(r, 0x0..=0x8 | 0xe | 0xf).then_some((0xe800 | r, "POKEY"))
        }
        _ => None,
    }
}

/// Collapse an address set into ranges, for reading.
fn ranges(s: &BTreeSet<u16>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut it = s.iter().copied().peekable();
    while let Some(lo) = it.next() {
        let mut hi = lo;
        while it.peek() == Some(&(hi.wrapping_add(1))) {
            hi = it.next().unwrap();
        }
        out.push(if lo == hi { format!("{lo:04X}") } else { format!("{lo:04X}-{hi:04X}") });
    }
    out.join(" ")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Writer {
    None,
    BiosBoot,
    BiosRun,
    Other,
}

struct Census {
    entered: Option<u64>,
    wait_start: Option<u64>,
    last_fetch: u16,
    /// Opcode of the previous instruction, to tell a return from a call.
    last_op: u8,
    /// RTI/RTS landing in the BIOS: resuming an interrupted BIOS
    /// instruction (an NMI taken right after an IRQ entry, say), not a call.
    returns_into_bios: u64,
    pending_vector: Option<u16>,
    calls: BTreeMap<u16, u64>,
    vectors: BTreeMap<u16, u64>,
    exits: BTreeMap<(u16, u16), u64>,
    data_reads: BTreeMap<u16, u64>,
    dma_bios: u64,
    dma_bios_lo: u16,
    dma_bios_hi: u16,
    bios_pcs_after_entry: BTreeSet<u16>,
    writer: Vec<Writer>,
    ram_dep_boot: BTreeSet<u16>,
    ram_dep_run: BTreeSet<u16>,
    ram_dep_never: BTreeSet<u16>,
    stack_dep: u64,
    regs_boot: BTreeMap<u16, (&'static str, u8)>,
    regs_bios_run: BTreeSet<u16>,
    regs_cart: BTreeSet<u16>,
    cart_pcs: BTreeSet<u16>,
    /// BRKs executed outside the BIOS: a wrong mapping runs into zeros.
    brks: u64,
}

impl Census {
    fn new() -> Self {
        Census {
            entered: None,
            wait_start: None,
            last_fetch: 0,
            last_op: 0,
            returns_into_bios: 0,
            pending_vector: None,
            calls: BTreeMap::new(),
            vectors: BTreeMap::new(),
            exits: BTreeMap::new(),
            data_reads: BTreeMap::new(),
            dma_bios: 0,
            dma_bios_lo: 0xffff,
            dma_bios_hi: 0,
            bios_pcs_after_entry: BTreeSet::new(),
            writer: vec![Writer::None; 0x4000],
            ram_dep_boot: BTreeSet::new(),
            ram_dep_run: BTreeSet::new(),
            ram_dep_never: BTreeSet::new(),
            stack_dep: 0,
            regs_boot: BTreeMap::new(),
            regs_bios_run: BTreeSet::new(),
            regs_cart: BTreeSet::new(),
            cart_pcs: BTreeSet::new(),
            brks: 0,
        }
    }

    fn observe(&mut self, a: &Access) {
        let entered = self.entered.is_some();
        match a.kind {
            AccessKind::Dma => {
                if entered && in_bios(a.addr) {
                    self.dma_bios += 1;
                    self.dma_bios_lo = self.dma_bios_lo.min(a.addr);
                    self.dma_bios_hi = self.dma_bios_hi.max(a.addr);
                }
            }
            AccessKind::Fetch => {
                let from = self.last_fetch;
                let from_op = self.last_op;
                self.last_fetch = a.addr;
                self.last_op = a.value;
                if self.wait_start.is_none() && a.addr == WAIT_LOOP {
                    self.wait_start = Some(a.cycle);
                }
                if !entered && in_cart(a.addr) {
                    self.entered = Some(a.cycle);
                }
                if self.entered.is_none() {
                    return;
                }
                if a.value == 0x00 && !in_bios(a.addr) {
                    self.brks += 1;
                }
                if in_cart(a.addr) {
                    self.cart_pcs.insert(a.addr);
                }
                if in_bios(a.addr) {
                    self.bios_pcs_after_entry.insert(a.addr);
                }
                if let Some(v) = self.pending_vector.take() {
                    // First fetch after an interrupt entry: the handler.
                    *self.vectors.entry(v).or_default() += 1;
                } else if in_bios(a.addr) && !in_bios(from) && matches!(from_op, 0x40 | 0x60) {
                    self.returns_into_bios += 1;
                } else if in_bios(a.addr) && !in_bios(from) {
                    *self.calls.entry(a.addr).or_default() += 1;
                } else if !in_bios(a.addr) && in_bios(from) {
                    // Leaving the BIOS: from which instruction, to where.
                    *self.exits.entry((from, a.addr)).or_default() += 1;
                }
            }
            AccessKind::Read => {
                if a.interrupt {
                    if a.addr >= 0xfffa {
                        self.pending_vector = Some(a.addr & 0xfffe);
                    }
                    return;
                }
                if !entered {
                    return;
                }
                let by_bios = in_bios(a.pc);
                if !by_bios && in_bios(a.addr) {
                    *self.data_reads.entry(a.addr).or_default() += 1;
                }
                if by_bios || a.addr >= 0x4000 {
                    return;
                }
                let w = self.writer[a.addr as usize];
                if (0x0100..0x0200).contains(&a.addr) {
                    if matches!(w, Writer::BiosBoot | Writer::BiosRun) {
                        self.stack_dep += 1;
                    }
                    return;
                }
                match w {
                    Writer::BiosBoot => {
                        self.ram_dep_boot.insert(a.addr);
                    }
                    Writer::BiosRun => {
                        self.ram_dep_run.insert(a.addr);
                    }
                    Writer::None => {
                        self.ram_dep_never.insert(a.addr);
                    }
                    Writer::Other => {}
                }
            }
            AccessKind::Write => {
                let by_bios = in_bios(a.pc);
                if a.addr < 0x4000 {
                    self.writer[a.addr as usize] = match (by_bios, entered) {
                        (true, false) => Writer::BiosBoot,
                        (true, true) => Writer::BiosRun,
                        (false, _) => Writer::Other,
                    };
                } else if let Some((r, chip)) = canon_reg(a.addr) {
                    if by_bios && !entered {
                        self.regs_boot.insert(r, (chip, a.value));
                    } else if by_bios {
                        self.regs_bios_run.insert(r);
                    } else {
                        self.regs_cart.insert(r);
                    }
                }
            }
        }
    }
}

struct Shared(Arc<Mutex<Census>>);

impl Probe for Shared {
    fn access(&mut self, a: &Access) {
        self.0.lock().unwrap().observe(a);
    }
}

struct Outcome {
    title: String,
    mapping: Mapping,
    skip: bool,
    halted: bool,
    c: Census,
}

/// The scripted input: the same for every title, a 240-frame cycle from frame
/// 300 (after the longest boot). Start once, fire twice, the top button,
/// keypad 1, and the stick sweeping both axes through its range throughout.
/// Generic on purpose: it is a coverage probe, not a play-through.
fn script(m: &mut Machine, f: u64) {
    if f < 300 {
        return;
    }
    let phase = (f - 300) % 240;
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

fn run(path: &Path, mapping: Mapping, bios: &[u8], frames: u64, input: bool) -> Outcome {
    let rom = std::fs::read(path).expect("read cart");
    let skip = rom.len() >= 3 && rom[rom.len() - 3] == 0xff;
    let cart = Cart::new(rom, mapping).expect("mapping");
    let census = Arc::new(Mutex::new(Census::new()));
    let mut m = Machine::new(cart, Some(bios));
    // Attached after reset: the census starts at the first instruction.
    m.bus.probe = Some(Box::new(Shared(census.clone())));
    for f in 0..frames {
        if input {
            script(&mut m, f);
        }
        m.run_frame();
        if m.cpu.halted {
            break;
        }
    }
    let halted = m.cpu.halted;
    drop(m);
    let c = Arc::try_unwrap(census).ok().unwrap().into_inner().unwrap();
    Outcome {
        title: path.file_name().unwrap().to_string_lossy().into_owned(),
        mapping,
        skip,
        halted,
        c,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dir = root.join("dumps/Atari 5200 ROM Collection/Commerical Games (1982-1984)");
    let mut frames = 1800u64;
    let mut input = false;
    let mut out = root.join("out/census.txt");
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => {
                i += 1;
                dir = PathBuf::from(&args[i]);
            }
            "--frames" => {
                i += 1;
                frames = args[i].parse().expect("frames");
            }
            "--input" => input = true,
            "--out" => {
                i += 1;
                out = PathBuf::from(&args[i]);
            }
            a => panic!("unknown argument {a}"),
        }
        i += 1;
    }
    let bios = std::fs::read(root.join("bios/[BIOS] Atari 5200 (USA).a52")).expect("read bios");

    let mut jobs: Vec<(PathBuf, Mapping)> = Vec::new();
    let mut unsupported = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).expect("read dir").map(|e| e.unwrap().path()).collect();
    files.sort();
    for p in &files {
        let len = std::fs::metadata(p).unwrap().len() as usize;
        let cands = Mapping::candidates(len);
        if cands.is_empty() {
            unsupported.push((p.file_name().unwrap().to_string_lossy().into_owned(), len));
        }
        for &m in cands {
            jobs.push((p.clone(), m));
        }
    }

    let jobs = Arc::new(Mutex::new(jobs.into_iter().enumerate().collect::<Vec<_>>()));
    let results = Arc::new(Mutex::new(Vec::new()));
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let mut hs = Vec::new();
    for _ in 0..threads {
        let (jobs, results, bios) = (jobs.clone(), results.clone(), bios.clone());
        hs.push(std::thread::spawn(move || loop {
            let job = jobs.lock().unwrap().pop();
            let Some((idx, (path, m))) = job else { break };
            let o = run(&path, m, &bios, frames, input);
            results.lock().unwrap().push((idx, o));
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
    let mut results = Arc::try_unwrap(results).ok().unwrap().into_inner().unwrap();
    results.sort_by_key(|(i, _)| *i);

    let mut r = String::new();
    let fr = |cyc: u64| cyc as f64 / CYCLES_PER_FRAME;
    writeln!(r, "BIOS census: {} runs over {} files in {}, {frames} frames each, {}", results.len(), files.len(), dir.display(), if input { "scripted input (start, fire, top, keypad 1, stick sweep)" } else { "no input" }).unwrap();
    for (name, len) in &unsupported {
        writeln!(r, "UNSUPPORTED SIZE: {name} ({len} bytes)").unwrap();
    }
    for (_, o) in &results {
        let c = &o.c;
        writeln!(r).unwrap();
        writeln!(r, "== {} [{:?}]", o.title, o.mapping).unwrap();
        let entry = c.entered.map(|e| format!("cycle {e} (frame {:.1})", fr(e))).unwrap_or_else(|| "NEVER".into());
        let setup = match (c.wait_start, c.entered) {
            (Some(w), Some(e)) => format!("setup {w} cycles, wait {} cycles", e - w),
            _ => String::from("no logo wait"),
        };
        writeln!(
            r,
            "  boot     : {} ; entered {entry} ; {setup}{}",
            if o.skip { "SKIP ($BFFD=FF)" } else { "FULL" },
            if o.halted { " ; CPU JAMMED" } else { "" }
        )
        .unwrap();
        writeln!(r, "  cart pcs : {} distinct ; {} BRKs executed outside the BIOS", c.cart_pcs.len(), c.brks).unwrap();
        writeln!(r, "  calls    : {}", c.calls.iter().map(|(a, n)| format!("{a:04X}x{n}")).collect::<Vec<_>>().join(" ")).unwrap();
        writeln!(
            r,
            "  vectors  : {}",
            c.vectors
                .iter()
                .map(|(a, n)| format!("{}x{n}", if *a == 0xfffa { "NMI" } else { "IRQ/BRK" }))
                .collect::<Vec<_>>()
                .join(" ")
        )
        .unwrap();
        let mut exits: BTreeMap<u16, BTreeSet<u16>> = BTreeMap::new();
        for (f, t) in c.exits.keys() {
            exits.entry(*f).or_default().insert(*t);
        }
        writeln!(
            r,
            "  exits    : {}",
            exits.iter().map(|(f, ts)| format!("{f:04X}->[{}]", ranges(ts))).collect::<Vec<_>>().join(" ")
        )
        .unwrap();
        writeln!(r, "  bios pcs : {} distinct after entry", c.bios_pcs_after_entry.len()).unwrap();
        writeln!(r, "  data     : {} reads of [{}]", c.data_reads.values().sum::<u64>(), ranges(&c.data_reads.keys().copied().collect())).unwrap();
        writeln!(
            r,
            "  dma      : {} ANTIC fetches of the BIOS region{}",
            c.dma_bios,
            if c.dma_bios > 0 { format!(" ({:04X}-{:04X})", c.dma_bios_lo, c.dma_bios_hi) } else { String::new() }
        )
        .unwrap();
        writeln!(r, "  ram boot : {} [{}]", c.ram_dep_boot.len(), ranges(&c.ram_dep_boot)).unwrap();
        writeln!(r, "  ram run  : {} [{}]", c.ram_dep_run.len(), ranges(&c.ram_dep_run)).unwrap();
        writeln!(r, "  ram never: {} bytes read before anyone wrote them", c.ram_dep_never.len()).unwrap();
        writeln!(r, "  stack    : {} cart reads of stack bytes a BIOS handler pushed", c.stack_dep).unwrap();
        let inherited: Vec<String> = c
            .regs_boot
            .iter()
            .filter(|(a, _)| !c.regs_cart.contains(a))
            .map(|(a, (chip, v))| format!("{chip}:{a:04X}={v:02X}"))
            .collect();
        writeln!(r, "  regs     : {} stateful registers the BIOS set at boot and the cart never wrote [{}]", inherited.len(), inherited.join(" ")).unwrap();
        writeln!(r, "  bios regs: BIOS code wrote [{}] after entry", ranges(&c.regs_bios_run)).unwrap();
    }

    // Aggregate, one row per FILE. For 16K the mapping that kept running and
    // ran the most distinct cart code wins; the per-title rows show both.
    let mut best: BTreeMap<String, &Outcome> = BTreeMap::new();
    for (_, o) in &results {
        let e = best.entry(o.title.clone()).or_insert(o);
        if (!o.halted, o.c.brks == 0, o.c.cart_pcs.len()) > (!e.halted, e.c.brks == 0, e.c.cart_pcs.len()) {
            *e = o;
        }
    }
    let n = best.len();
    let count = |f: &dyn Fn(&Outcome) -> bool| best.values().filter(|o| f(o)).count();
    writeln!(r).unwrap();
    writeln!(r, "== AGGREGATE over {n} files").unwrap();
    writeln!(r, "  entered the cart          : {}", count(&|o| o.c.entered.is_some())).unwrap();
    writeln!(r, "  still running at the end  : {}", count(&|o| !o.halted)).unwrap();
    writeln!(r, "  running with no BRK storm : {}", count(&|o| !o.halted && o.c.brks == 0)).unwrap();
    for o in best.values().filter(|o| o.c.brks > 0) {
        writeln!(r, "    BRKs under its best mapping ({}): {}", o.c.brks, o.title).unwrap();
    }
    for o in best.values().filter(|o| o.halted) {
        writeln!(r, "    JAMMED under every mapping tried: {}", o.title).unwrap();
    }
    let mut maps: BTreeMap<String, usize> = BTreeMap::new();
    for o in best.values() {
        *maps.entry(format!("{:?}", o.mapping)).or_default() += 1;
    }
    writeln!(r, "  mapping that ran          : {}", maps.iter().map(|(m, n)| format!("{m}:{n}")).collect::<Vec<_>>().join(" ")).unwrap();
    for o in best.values().filter(|o| o.mapping == Mapping::OneChip16) {
        writeln!(r, "    one-chip 16K: {}", o.title).unwrap();
    }
    let both_ran = best
        .values()
        .filter(|o| {
            o.mapping == Mapping::TwoChip16 || o.mapping == Mapping::OneChip16
        })
        .filter(|o| results.iter().filter(|(_, x)| x.title == o.title && !x.halted && x.c.brks == 0).count() > 1)
        .count();
    writeln!(r, "  16K titles that ran under BOTH mappings (undecided) : {both_ran}").unwrap();
    writeln!(r, "  skip boot ($BFFD=FF)      : {}", count(&|o| o.skip)).unwrap();
    writeln!(r, "  call into the BIOS        : {}", count(&|o| !o.c.calls.is_empty())).unwrap();
    writeln!(r, "  read the BIOS as data     : {}", count(&|o| !o.c.data_reads.is_empty())).unwrap();
    let font = count(&|o| o.c.data_reads.keys().any(|a| *a < 0xfc00));
    let code = count(&|o| o.c.data_reads.keys().any(|a| *a >= 0xfc00));
    writeln!(r, "    of the font $F800-$FBFF : {font} ; of code/vectors $FC00-$FFFF : {code}").unwrap();
    writeln!(r, "  ANTIC fetches BIOS (font) : {}", count(&|o| o.c.dma_bios > 0)).unwrap();
    writeln!(r, "  take the NMI vector       : {}", count(&|o| o.c.vectors.contains_key(&0xfffa))).unwrap();
    writeln!(r, "  take the IRQ/BRK vector   : {}", count(&|o| o.c.vectors.contains_key(&0xfffe))).unwrap();
    writeln!(r, "  read BIOS-boot RAM        : {}", count(&|o| !o.c.ram_dep_boot.is_empty())).unwrap();
    writeln!(r, "  read BIOS-runtime RAM     : {}", count(&|o| !o.c.ram_dep_run.is_empty())).unwrap();
    writeln!(r, "  pull what a BIOS handler pushed : {}", count(&|o| o.c.stack_dep > 0)).unwrap();
    writeln!(r, "  BIOS writes hw regs after entry : {}", count(&|o| !o.c.regs_bios_run.is_empty())).unwrap();
    let union: BTreeSet<u16> = best.values().flat_map(|o| o.c.bios_pcs_after_entry.iter().copied()).collect();
    writeln!(r, "  BIOS instructions executed after entry, union : {} [{}]", union.len(), ranges(&union)).unwrap();

    let mut call_targets: BTreeMap<u16, usize> = BTreeMap::new();
    let mut exit_points: BTreeMap<u16, usize> = BTreeMap::new();
    let mut boot_ram: BTreeMap<u16, usize> = BTreeMap::new();
    let mut run_ram: BTreeMap<u16, usize> = BTreeMap::new();
    let mut regvals: BTreeMap<(u16, u8), usize> = BTreeMap::new();
    let mut data_addrs: BTreeMap<u16, usize> = BTreeMap::new();
    let mut bios_run_regs: BTreeMap<u16, usize> = BTreeMap::new();
    for o in best.values() {
        for a in o.c.calls.keys() {
            *call_targets.entry(*a).or_default() += 1;
        }
        for f in o.c.exits.keys().map(|(f, _)| *f).collect::<BTreeSet<_>>() {
            *exit_points.entry(f).or_default() += 1;
        }
        for a in &o.c.ram_dep_boot {
            *boot_ram.entry(*a).or_default() += 1;
        }
        for a in &o.c.ram_dep_run {
            *run_ram.entry(*a).or_default() += 1;
        }
        for (a, (_, v)) in o.c.regs_boot.iter().filter(|(a, _)| !o.c.regs_cart.contains(a)) {
            *regvals.entry((*a, *v)).or_default() += 1;
        }
        for a in o.c.data_reads.keys() {
            *data_addrs.entry(*a).or_default() += 1;
        }
        for a in &o.c.regs_bios_run {
            *bios_run_regs.entry(*a).or_default() += 1;
        }
    }
    let top = |m: &BTreeMap<u16, usize>| {
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        v.iter().take(40).map(|(a, n)| format!("{a:04X}:{n}")).collect::<Vec<_>>().join(" ")
    };
    let common = |m: &BTreeMap<u16, usize>, k: usize| ranges(&m.iter().filter(|(_, n)| **n >= k).map(|(a, _)| *a).collect());
    writeln!(r, "  call targets (titles)     : {}", top(&call_targets)).unwrap();
    writeln!(r, "  BIOS exit points (titles) : {}", top(&exit_points)).unwrap();
    writeln!(r, "  BIOS data read by >=2 titles : {}", common(&data_addrs, 2)).unwrap();
    writeln!(r, "  BIOS data read by 1 title    : {}", ranges(&data_addrs.iter().filter(|(_, n)| **n == 1).map(|(a, _)| *a).collect())).unwrap();
    writeln!(r, "  boot RAM read by >=5 titles  : {}", common(&boot_ram, 5)).unwrap();
    writeln!(r, "  boot RAM read by >=2 titles  : {}", common(&boot_ram, 2)).unwrap();
    writeln!(r, "  runtime RAM read (titles) : {}", top(&run_ram)).unwrap();
    writeln!(r, "  BIOS-written hw regs after entry (titles) : {}", top(&bios_run_regs)).unwrap();
    let mut rv: Vec<_> = regvals.iter().filter(|((_, v), _)| *v != 0).collect();
    rv.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    writeln!(
        r,
        "  inherited NONZERO regs (value:titles) : {}",
        rv.iter().map(|((a, v), n)| format!("{a:04X}={v:02X}:{n}")).collect::<Vec<_>>().join(" ")
    )
    .unwrap();
    let mut setups: BTreeSet<(u64, u64)> = BTreeSet::new();
    for o in best.values() {
        if let (Some(w), Some(e)) = (o.c.wait_start, o.c.entered) {
            setups.insert((w, e - w));
        }
    }
    for (s, w) in &setups {
        writeln!(
            r,
            "  full boot: setup {s} cycles ({:.2} frames), visible wait {w} cycles ({:.2} frames), setup share {:.2}%",
            fr(*s),
            fr(*w),
            100.0 * *s as f64 / (*s + *w) as f64
        )
        .unwrap();
    }

    std::fs::create_dir_all(out.parent().unwrap()).ok();
    std::fs::write(&out, &r).expect("write report");
    let agg = r.split("== AGGREGATE").nth(1).unwrap_or("");
    println!("{}", r.lines().next().unwrap());
    for (name, len) in &unsupported {
        println!("UNSUPPORTED SIZE: {name} ({len} bytes)");
    }
    println!("== AGGREGATE{agg}");
    println!("full report: {}", out.display());
}
