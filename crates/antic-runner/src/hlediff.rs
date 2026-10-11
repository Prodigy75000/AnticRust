// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `hlediff`: every title on the real BIOS and on the HLE, compared.
//!
//!   hlediff [--dir <folder>] [--frames N] [--only <substring>] [--swap]
//!
//! The real BIOS is the oracle. Each title runs twice: on the dump and on
//! the HLE image. Runs are aligned at the cartridge's first instruction (the
//! HLE skips the 4.25 s logo, so wall-clock frames cannot line up), then
//! advanced to the next frame boundary, then driven by the same scripted
//! input counted from there. Frames and RAM are compared at checkpoints.
//!
//! With --swap both runs boot on the REAL BIOS and the HLE image replaces it
//! at the cartridge's first instruction. Boot then cannot differ, so what is
//! left is purely the HLE's run-time behaviour: dispatchers, VBI, keypad,
//! font, tables.
//!
//! A difference is a lead, not a verdict: POKEY's RANDOM depends on how long
//! the machine has been running, and the HLE gets there sooner, so a game
//! that seeds from it legitimately diverges. Differing titles get a
//! side-by-side PNG (real left, HLE right) in out/hlediff/ to look at.

use antic_core::cart::{Cart, Mapping};
use antic_core::gtia::{HEIGHT, WIDTH};
use antic_core::machine::Machine;
use antic_core::pokey::Key;
use std::path::PathBuf;

const CHECKPOINTS: [u64; 6] = [1, 30, 120, 300, 900, 1800];

/// The same generic input as the census, counted from the aligned frame.
fn script(m: &mut Machine, f: u64) {
    if f < 40 {
        return;
    }
    let phase = (f - 40) % 240;
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

/// Boot to the cartridge's first instruction, then to the next frame start.
/// None if the cart is never entered.
fn boot(rom: &[u8], mapping: Mapping, bios: Option<&[u8]>, swap: bool) -> Option<Machine> {
    let mut m = Machine::new(Cart::new(rom.to_vec(), mapping).ok()?, bios);
    let mut steps = 0u64;
    while m.cpu.pc < 0x4000 || m.cpu.pc >= 0xc000 {
        m.step();
        steps += 1;
        if steps > 20_000_000 {
            return None;
        }
    }
    if swap {
        m.bus.bios = Some(Box::new(antic_core::hle::image()));
    }
    m.run_frame();
    Some(m)
}

/// Distinct cartridge addresses executed in 600 frames on the real BIOS
/// with the scripted input: the census's tie-break between mappings.
fn coverage(rom: &[u8], mapping: Mapping, bios: &[u8]) -> usize {
    let Some(mut m) = boot(rom, mapping, Some(bios), false) else { return 0 };
    let mut seen = std::collections::HashSet::new();
    let mut brk = false;
    for f in 0..600 {
        script(&mut m, f);
        let end = m.bus.antic.frame;
        while m.bus.antic.frame == end {
            let pc = m.cpu.pc;
            if (0x4000..0xc000).contains(&pc) {
                seen.insert(pc);
                if m.bus.peek(pc) == Some(0) {
                    brk = true;
                }
            }
            m.step();
        }
        if m.cpu.halted {
            break;
        }
    }
    if brk { 0 } else { seen.len() }
}

struct Snap {
    frame: Vec<u8>,
    ram: Vec<u8>,
}

fn run(rom: &[u8], mapping: Mapping, bios: Option<&[u8]>, frames: u64, swap: bool) -> Option<(Vec<Snap>, bool)> {
    let mut m = boot(rom, mapping, bios, swap)?;
    let mut snaps = Vec::new();
    for f in 0..frames {
        script(&mut m, f);
        m.run_frame();
        if CHECKPOINTS.contains(&(f + 1)) {
            snaps.push(Snap { frame: m.bus.gtia.frame.clone(), ram: m.bus.ram.to_vec() });
        }
        if m.cpu.halted {
            return Some((snaps, true));
        }
    }
    Some((snaps, false))
}

fn save_pair(path: &PathBuf, a: &[u8], b: &[u8]) {
    let pal = antic_core::gtia::palette();
    let w = WIDTH * 2 + 8;
    let mut rgb = vec![0u8; w * HEIGHT * 3];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let o = (y * w + x) * 3;
            rgb[o..o + 3].copy_from_slice(&pal[a[y * WIDTH + x] as usize]);
            let o = (y * w + WIDTH + 8 + x) * 3;
            rgb[o..o + 3].copy_from_slice(&pal[b[y * WIDTH + x] as usize]);
        }
    }
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, HEIGHT as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgb).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut dir = root.join("dumps/Atari 5200 ROM Collection/Commerical Games (1982-1984)");
    let mut frames = 1800u64;
    let mut only = String::new();
    let mut swap = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => { i += 1; dir = PathBuf::from(&args[i]); }
            "--frames" => { i += 1; frames = args[i].parse().unwrap(); }
            "--only" => { i += 1; only = args[i].clone(); }
            "--swap" => swap = true,
            a => panic!("unknown argument {a}"),
        }
        i += 1;
    }
    let bios = std::fs::read(root.join("bios/[BIOS] Atari 5200 (USA).a52")).expect("read bios");
    let outdir = root.join("out/hlediff");
    std::fs::create_dir_all(&outdir).ok();

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    files.retain(|p| p.to_string_lossy().contains(&only));

    let results = std::sync::Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(p) = files.get(k) else { break };
                let rom = std::fs::read(p).unwrap();
                let title = p.file_name().unwrap().to_string_lossy().into_owned();
                // The mapping the real BIOS runs best: not jammed, then the
                // most distinct cart code, as the census decides it.
                let mut chosen: Option<(Mapping, Vec<Snap>, (bool, usize))> = None;
                for &mp in Mapping::candidates(rom.len()) {
                    if let Some((snaps, jam)) = run(&rom, mp, Some(&bios), frames, false) {
                        let score = (!jam, coverage(&rom, mp, &bios));
                        if chosen.as_ref().map_or(true, |c| score > c.2) {
                            chosen = Some((mp, snaps, score));
                        }
                    }
                }
                let chosen = chosen.map(|(m, s, _)| (m, s));
                let line = match chosen {
                    None => format!("SKIP  {title}: no mapping runs on the real BIOS"),
                    Some((mp, real)) => match if swap { run(&rom, mp, Some(&bios), frames, true) } else { run(&rom, mp, None, frames, false) } {
                        None => format!("FAIL  {title} [{mp:?}]: HLE never entered the cart"),
                        Some((hle, jam)) => {
                            let same_frames: Vec<bool> = real.iter().zip(&hle).map(|(a, b)| a.frame == b.frame).collect();
                            let ram_diff: Vec<usize> = real
                                .iter()
                                .zip(&hle)
                                .map(|(a, b)| a.ram.iter().zip(&b.ram).filter(|(x, y)| x != y).count())
                                .collect();
                            let marks: String = same_frames.iter().map(|&s| if s { '=' } else { 'x' }).collect();
                            let all = same_frames.iter().all(|&s| s) && hle.len() == real.len();
                            if !all {
                                if let (Some(a), Some(b)) = (real.last(), hle.last()) {
                                    let name: String = title.chars().filter(|c| c.is_ascii_alphanumeric()).take(28).collect();
                                    save_pair(&outdir.join(format!("{name}.png")), &a.frame, &b.frame);
                                }
                            }
                            format!(
                                "{} {title} [{mp:?}] frames {marks} ram-diff {:?}{}",
                                if all { "SAME " } else if jam { "JAM  " } else { "DIFF " },
                                ram_diff,
                                if jam { " (HLE run jammed)" } else { "" }
                            )
                        }
                    },
                };
                results.lock().unwrap().push((k, line));
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort();
    let mut same = 0;
    for (_, l) in &results {
        println!("{l}");
        if l.starts_with("SAME") {
            same += 1;
        }
    }
    println!("checkpoints at frames {CHECKPOINTS:?} after alignment; = identical frame, x differs");
    println!("{same} of {} titles identical at every checkpoint ({})", results.len(), if swap { "run time only: both booted on the real BIOS, HLE swapped in at entry" } else { "HLE boot and run time" });
}
