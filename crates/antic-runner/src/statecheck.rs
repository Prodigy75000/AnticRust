// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `statecheck`: does a save state restore a game exactly, for every game?
//!
//!   statecheck [--at N] [--after M]
//!
//! For every file in the three corpus folders, on the HLE with the census's
//! scripted input: run to frame N and save; keep running M frames, keeping a
//! checksum of every frame and the RAM at the end. Then a FRESH machine,
//! first run for a different number of frames so it is somewhere else
//! entirely, loads the state and plays the same M frames with the same input.
//! Every frame, the RAM, and the final state must match.

use antic_core::cart::Cart;
use antic_core::machine::{Machine, STATE_SIZE};
use antic_core::pokey::Key;
use std::path::PathBuf;

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

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| {
        (h ^ x as u64).wrapping_mul(0x100_0000_01b3)
    })
}

/// Play frames from..to with the script; return each frame's checksum.
fn play(m: &mut Machine, from: u64, to: u64) -> Vec<u64> {
    (from..to)
        .map(|f| {
            script(m, f);
            m.run_frame();
            fnv(&m.bus.gtia.frame)
        })
        .collect()
}

fn check(rom: &[u8], at: u64, after: u64) -> Result<(), String> {
    let new = || {
        Cart::identified(rom.to_vec())
            .map(|c| Machine::new(c, None))
            .map_err(|e| format!("{e:?}"))
    };
    let mut a = new()?;
    play(&mut a, 0, at);
    let state = a.save_state();
    if state.len() != STATE_SIZE {
        return Err(format!("state is {} bytes, not {STATE_SIZE}", state.len()));
    }
    let frames_a = play(&mut a, at, at + after);

    let mut b = new()?;
    play(&mut b, 0, at / 3 + 17);
    b.load_state(&state)
        .map_err(|e| format!("load refused: {e:?}"))?;
    if b.save_state() != state {
        return Err("state does not round-trip".into());
    }
    let frames_b = play(&mut b, at, at + after);

    if let Some(k) = frames_a.iter().zip(&frames_b).position(|(x, y)| x != y) {
        return Err(format!("frame {} differs after restore", at + k as u64));
    }
    if a.bus.ram[..] != b.bus.ram[..] {
        return Err("RAM differs at the end".into());
    }
    if a.save_state() != b.save_state() {
        return Err("final state differs".into());
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |name: &str, d: u64| {
        args.iter()
            .position(|a| a == name)
            .map(|k| args[k + 1].parse().unwrap())
            .unwrap_or(d)
    };
    let (at, after) = (get("--at", 600), get("--after", 300));
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dumps/Atari 5200 ROM Collection");
    let mut files = Vec::new();
    for dir in std::fs::read_dir(&root).unwrap() {
        let dir = dir.unwrap().path();
        if dir.is_dir() {
            for f in std::fs::read_dir(&dir).unwrap() {
                files.push(f.unwrap().path());
            }
        }
    }
    files.sort();

    let results = std::sync::Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(p) = files.get(k) else { break };
                let rom = std::fs::read(p).unwrap();
                let r = check(&rom, at, after);
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
                results.lock().unwrap().push((k, name, r));
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|r| r.0);
    let mut ok = 0;
    let mut skipped = 0;
    for (_, name, r) in &results {
        match r {
            Ok(()) => ok += 1,
            Err(e) if e.starts_with("UnsupportedSize") => {
                skipped += 1;
                println!("SKIP  {name}: no mapping for its size");
            }
            Err(e) => println!("FAIL  {name}: {e}"),
        }
    }
    println!(
        "{ok} of {} files restore exactly (saved at frame {at}, {after} frames compared){}",
        results.len() - skipped,
        if skipped > 0 {
            format!("; {skipped} skipped")
        } else {
            String::new()
        }
    );
}
