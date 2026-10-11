// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `rahash`: the proposed RetroAchievements hash over the whole corpus, and
//! the check that it never merges two images that run differently.
//!
//!   rahash [--frames N]
//!   rahash --list            <md5 of canonical image> <path>, one per file
//!
//! For every file in the three corpus folders: MD5 of the file, MD5 of
//! `hash::canonical`, and which files now share a hash. Then, for every file
//! the canonical form changes, both images run on the HLE with the same
//! scripted input and are compared frame by frame and in RAM. A rule that
//! merged two images which behave differently would show up here.

use antic_core::cart::Cart;
use antic_core::cart::Mapping;
use antic_core::hash::{canonical, reduce};
use antic_core::machine::Machine;
use antic_core::pokey::Key;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn md5hex(b: &[u8]) -> String {
    format!("{:x}", md5::compute(b))
}

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

/// Frame and RAM fingerprints at checkpoints, on the HLE. `mapping` None
/// means the core's usual identification.
fn trace(rom: &[u8], mapping: Option<Mapping>, frames: u64) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
    let cart = match mapping {
        Some(mp) => Cart::new(rom.to_vec(), mp).ok()?,
        None => Cart::identified(rom.to_vec()).ok()?,
    };
    let mut m = Machine::new(cart, None);
    let mut out = Vec::new();
    for f in 0..frames {
        script(&mut m, f);
        m.run_frame();
        if (f + 1) % 300 == 0 {
            out.push((m.bus.gtia.frame.clone(), m.bus.ram.to_vec()));
        }
    }
    Some(out)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let frames: u64 = args
        .iter()
        .position(|a| a == "--frames")
        .map(|k| args[k + 1].parse().unwrap())
        .unwrap_or(1800);
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dumps/Atari 5200 ROM Collection");
    let mut files = Vec::new();
    for dir in std::fs::read_dir(&root).unwrap() {
        let dir = dir.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&dir).unwrap() {
            files.push(f.unwrap().path());
        }
    }
    files.sort();
    if args.iter().any(|a| a == "--list") {
        for p in &files {
            println!(
                "{} {}",
                md5hex(&canonical(&std::fs::read(p).unwrap())),
                p.display()
            );
        }
        return;
    }

    let mut by_raw: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut by_canon: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut changed = Vec::new();
    for p in &files {
        let rom = std::fs::read(p).unwrap();
        let canon = canonical(&rom);
        let name = format!(
            "{}/{}",
            p.parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .chars()
                .take(4)
                .collect::<String>(),
            p.file_name().unwrap().to_string_lossy()
        );
        by_raw.entry(md5hex(&rom)).or_default().push(name.clone());
        by_canon
            .entry(md5hex(&canon))
            .or_default()
            .push(name.clone());
        if canon != rom {
            changed.push((name, rom, canon));
        }
    }
    println!(
        "{} files: {} distinct file hashes, {} distinct canonical hashes",
        files.len(),
        by_raw.len(),
        by_canon.len()
    );
    println!(
        "\ncanonical form differs from the file for {} files:",
        changed.len()
    );
    for (name, rom, canon) in &changed {
        println!("  {:6} -> {:6}  {name}", rom.len(), canon.len());
    }
    println!("\nhashes shared by more than one file (canonical):");
    for (h, names) in &by_canon {
        if names.len() > 1 {
            let raw_same = by_raw.get(h).map_or(0, |v| v.len());
            println!(
                "  {h}  ({} files, {} of them byte-identical to the canonical image)",
                names.len(),
                raw_same
            );
            for n in names {
                println!("      {n}");
            }
        }
    }

    println!(
        "\nbehaviour, original against canonical image, HLE, {frames} frames of scripted input:"
    );
    let mut same = 0;
    for (name, rom, canon) in &changed {
        // The canonical image runs under the mapping its reduction implies.
        let implied = reduce(rom).1;
        let (a, b) = (trace(rom, None, frames), trace(canon, implied, frames));
        let verdict = match (a, b) {
            (Some(a), Some(b)) if a == b => {
                same += 1;
                "identical".to_string()
            }
            (Some(a), Some(b)) => {
                let k = a.iter().zip(&b).position(|(x, y)| x != y).unwrap();
                format!("DIFFERS from checkpoint {} (frame {})", k, (k + 1) * 300)
            }
            _ => "could not load".to_string(),
        };
        println!("  {verdict:28} {name}");
    }
    println!(
        "{same} of {} changed files run identically from their canonical image",
        changed.len()
    );
}
