// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! `shot`: run a cartridge on the real BIOS and save frames as PNG.
//!
//!   shot <cart> [--frames N] [--out file.png] [--map mirror|one|two|linear|bob]
//!               [--press KEY@FRAME[+LEN]] [--every K] [--wav file.wav]
//!
//! KEY is start, pause, reset, star, hash, 0-9, fire or top (controller 1).
//! With --every K, frame N-1, N-1-K, ... are all written, suffixed.
//! Output defaults to out/shot.png (gitignored).

use antic_core::cart::{Cart, Mapping};
use antic_core::gtia::{HEIGHT, WIDTH};
use antic_core::machine::Machine;
use antic_core::pokey::Key;
use std::path::PathBuf;

struct Press {
    key: String,
    at: u64,
    len: u64,
}

fn apply(m: &mut Machine, key: &str, down: bool) {
    let c = &mut m.bus.pokey.controllers[0];
    let k = match key {
        "start" => Some(Key::Start),
        "pause" => Some(Key::Pause),
        "reset" => Some(Key::Reset),
        "star" => Some(Key::Star),
        "hash" => Some(Key::Hash),
        d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => Some(Key::Digit(d.as_bytes()[0] - b'0')),
        "fire" => {
            m.bus.gtia.trigger[0] = down;
            return;
        }
        "top" => {
            c.top = down;
            return;
        }
        other => panic!("unknown key {other}"),
    };
    c.key = if down { k } else { None };
}

fn save(m: &Machine, path: &PathBuf) {
    let rgb = m.frame_rgb();
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), WIDTH as u32, HEIGHT as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgb).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut cart_path = None;
    let mut frames = 300u64;
    let mut out = root.join("out/shot.png");
    let mut map = None;
    let mut presses = Vec::new();
    let mut every = 0u64;
    let mut wav: Option<PathBuf> = None;
    let mut audio: Vec<i16> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--frames" => { i += 1; frames = args[i].parse().expect("frames"); }
            "--out" => { i += 1; out = PathBuf::from(&args[i]); }
            "--every" => { i += 1; every = args[i].parse().expect("every"); }
            "--wav" => { i += 1; wav = Some(PathBuf::from(&args[i])); }
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
            "--press" => {
                i += 1;
                let (key, rest) = args[i].split_once('@').expect("KEY@FRAME");
                let (at, len) = match rest.split_once('+') {
                    Some((a, l)) => (a.parse().unwrap(), l.parse().unwrap()),
                    None => (rest.parse().unwrap(), 10),
                };
                presses.push(Press { key: key.to_string(), at, len });
            }
            a => cart_path = Some(a.to_string()),
        }
        i += 1;
    }
    let cart_path = cart_path.expect("usage: shot <cart> [options]");
    let rom = std::fs::read(&cart_path).expect("read cart");
    let bios = std::fs::read(root.join("bios/[BIOS] Atari 5200 (USA).a52")).expect("read bios");
    let cart = match map {
        Some(m) => Cart::new(rom, m).expect("mapping"),
        None => Cart::with_default_mapping(rom).expect("mapping"),
    };
    let mut m = Machine::new(cart, Some(&bios));
    // Sticks powered and centred is what a controller at rest reports.
    std::fs::create_dir_all(out.parent().unwrap()).ok();
    for f in 0..frames {
        for p in &presses {
            if f == p.at {
                apply(&mut m, &p.key, true);
            }
            if f == p.at + p.len {
                apply(&mut m, &p.key, false);
            }
        }
        m.run_frame();
        audio.extend(m.bus.pokey.take_samples());
        let last = f + 1 == frames;
        if last || (every > 0 && (frames - 1 - f) % every == 0) {
            let path = if last && every == 0 {
                out.clone()
            } else {
                out.with_file_name(format!("{}-{f:05}.png", out.file_stem().unwrap().to_string_lossy()))
            };
            save(&m, &path);
        }
    }
    if let Some(w) = &wav {
        write_wav(w, &audio);
        let peak = audio.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
        let rms = (audio.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / audio.len().max(1) as f64).sqrt();
        println!("wrote {} ({} samples, peak {peak}, rms {rms:.0})", w.display(), audio.len());
    }
    println!("wrote {} ({WIDTH}x{HEIGHT})", out.display());
}

/// Mono 16-bit PCM at the POKEY output rate.
fn write_wav(path: &PathBuf, s: &[i16]) {
    let rate = antic_core::pokey::SAMPLE_RATE;
    let data = (s.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for v in s {
        b.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, b).expect("write wav");
}
