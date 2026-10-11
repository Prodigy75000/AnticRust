// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! Klaus Dormann's 6502 functional test, as a verdict-printing gate.
//!
//! The binary is a 64 KiB image assembled with `disable_decimal = 0`, so it
//! also checks ADC/SBC in decimal mode over every valid BCD operand pair.
//! It ends by jumping to itself: at the success trap if everything passed,
//! anywhere else if a test failed. So the verdict is the PC the CPU gets stuck
//! at, and the listing names the failing test from that address.
//!
//! Vendored (gitignored) under `tests/vendor/klaus/`:
//!   cargo run --release -p antic-runner --bin klaus

use antic_core::cpu::{Cpu, CpuBus};
use std::path::PathBuf;

/// The `success` macro's `jmp *` in the upstream bin_files listing.
const SUCCESS_TRAP: u16 = 0x3469;
const ENTRY: u16 = 0x0400;
/// Far beyond the ~96M cycles a passing run takes; only a runaway hits it.
const CYCLE_LIMIT: u64 = 400_000_000;

struct Flat(Box<[u8; 0x10000]>);
impl CpuBus for Flat {
    fn read(&mut self, addr: u16) -> u8 {
        self.0[addr as usize]
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.0[addr as usize] = val;
    }
}

fn main() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/vendor/klaus/6502_functional_test.bin");
    let image = match std::fs::read(&path) {
        Ok(b) if b.len() == 0x10000 => b,
        Ok(b) => {
            eprintln!("{}: {} bytes, expected 65536", path.display(), b.len());
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!(
                "{}: {e}\nFetch it from https://github.com/Klaus2m5/6502_65C02_functional_tests/raw/master/bin_files/6502_functional_test.bin",
                path.display()
            );
            std::process::exit(2);
        }
    };
    let mut bus = Flat(Box::new([0; 0x10000]));
    bus.0.copy_from_slice(&image);
    let mut cpu = Cpu::new();
    cpu.pc = ENTRY;

    let mut instructions: u64 = 0;
    loop {
        let pc = cpu.pc;
        cpu.step(&mut bus);
        instructions += 1;
        if cpu.pc == pc {
            println!("== Klaus Dormann 6502 functional test ==");
            println!("trapped at ${pc:04X} after {instructions} instructions, {} cycles", cpu.cycles);
            if pc == SUCCESS_TRAP {
                println!("PASS");
                return;
            }
            println!(
                "FAIL: a=${:02X} x=${:02X} y=${:02X} p=${:02X} sp=${:02X}; look up ${pc:04X} in the listing",
                cpu.a, cpu.x, cpu.y, cpu.p, cpu.sp
            );
            std::process::exit(1);
        }
        if cpu.cycles > CYCLE_LIMIT {
            println!("FAIL: no trap within {CYCLE_LIMIT} cycles, pc=${:04X}", cpu.pc);
            std::process::exit(1);
        }
    }
}
