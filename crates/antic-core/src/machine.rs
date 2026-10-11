// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! The machine: CPU, bus, and the clock that ties ANTIC to both.
//!
//! Memory map (Altirra 13.4):
//!
//! | range         | device                                   |
//! |---------------|------------------------------------------|
//! | $0000-$3FFF   | 16K RAM                                  |
//! | $4000-$BFFF   | cartridge window                         |
//! | $C000-$CFFF   | GTIA, 32 registers mirrored              |
//! | $D400-$D4FF   | ANTIC, 16 registers mirrored             |
//! | $E800-$EFFF   | POKEY, 16 registers mirrored             |
//! | $F800-$FFFF   | BIOS (2K); $F800 is also the system font |
//!
//! Everything else is open bus: it reads the last value seen on the data bus.
//!
//! Every CPU access costs one machine cycle, and before it happens the bus
//! lets ANTIC take any cycle it planned (the CPU is halted for those), and
//! holds a CPU read while WSYNC has RDY low. Writes ignore RDY, as on the
//! real part.

use crate::antic::{Antic, DmaFetch, CYCLES_PER_LINE};
use crate::cart::Cart;
use crate::cpu::{Cpu, CpuBus, Pending};
use crate::gtia::Gtia;
use crate::pokey::Pokey;

/// What kind of bus cycle a probe is seeing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessKind {
    /// First byte of an instruction.
    Fetch,
    Read,
    Write,
    /// ANTIC fetch: display list, playfield, character set, player/missile.
    Dma,
}

/// One bus cycle, for a probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    pub addr: u16,
    pub value: u8,
    pub kind: AccessKind,
    /// Address of the instruction making the access, or of the instruction
    /// about to be interrupted while the CPU enters an interrupt.
    pub pc: u16,
    /// The CPU is entering an interrupt (the access is part of that sequence).
    pub interrupt: bool,
    /// Absolute machine cycle.
    pub cycle: u64,
}

/// Something that wants to see every bus cycle: the census, a tracer.
pub trait Probe {
    fn access(&mut self, a: &Access);
}

pub struct Bus {
    pub ram: Box<[u8; 0x4000]>,
    pub bios: Option<Box<[u8; 0x800]>>,
    pub cart: Cart,
    pub antic: Antic,
    pub gtia: Gtia,
    pub pokey: Pokey,
    /// Absolute machine cycle count.
    pub cycles: u64,
    /// WSYNC: CPU reads in [rdy_from, rdy_until) wait.
    rdy_from: u64,
    rdy_until: u64,
    /// Last value on the data bus, for open-bus reads.
    open_bus: u8,
    // Probe context, set by Machine::step.
    instr_pc: u16,
    first_access: bool,
    in_interrupt: bool,
    pub probe: Option<Box<dyn Probe>>,
}

impl Bus {
    fn new(cart: Cart, bios: Option<&[u8]>) -> Self {
        // No dump: the HLE image. A dump is the oracle, never a requirement.
        let mut img = Box::new([0u8; 0x800]);
        match bios {
            Some(b) => img.copy_from_slice(&b[b.len() - 0x800..]),
            None => *img = crate::hle::image(),
        }
        let bios = Some(img);
        let mut antic = Antic::new();
        antic.begin_line();
        Bus {
            ram: Box::new([0; 0x4000]),
            bios,
            cart,
            antic,
            gtia: Gtia::new(),
            pokey: Pokey::new(),
            cycles: 0,
            rdy_from: 0,
            rdy_until: 0,
            open_bus: 0xff,
            instr_pc: 0,
            first_access: false,
            in_interrupt: false,
            probe: None,
        }
    }

    /// Side-effect-free read, for DMA, disassembly and inspection.
    pub fn peek(&self, addr: u16) -> Option<u8> {
        match addr {
            0x0000..=0x3fff => Some(self.ram[addr as usize]),
            0x4000..=0xbfff => self.cart.peek(addr),
            0xf800..=0xffff => self.bios.as_ref().map(|b| b[(addr - 0xf800) as usize]),
            _ => None,
        }
    }

    fn read_mem(&mut self, addr: u16) -> u8 {
        let v = match addr {
            0x0000..=0x3fff => Some(self.ram[addr as usize]),
            0x4000..=0xbfff => {
                self.cart.access(addr);
                self.cart.peek(addr)
            }
            0xc000..=0xcfff => Some(self.gtia.read(addr)),
            0xd400..=0xd4ff => Some(self.antic.read(addr)),
            0xe800..=0xefff => {
                let (p, s) = (self.gtia.pots_powered(), self.gtia.selected_controller());
                Some(self.pokey.read(addr, p, s))
            }
            0xf800..=0xffff => self.bios.as_ref().map(|b| b[(addr - 0xf800) as usize]),
            _ => None,
        };
        let v = v.unwrap_or(self.open_bus);
        self.open_bus = v;
        v
    }

    fn write_mem(&mut self, addr: u16, v: u8) {
        self.open_bus = v;
        // Anything that can change the picture: draw up to now with the old
        // value first, so mid-line changes land where the hardware puts them.
        let visual = match addr {
            0xc000..=0xcfff => true,
            0xd400..=0xd4ff => matches!(addr & 0x0f, 0x00 | 0x01 | 0x04 | 0x09),
            _ => false,
        };
        if visual {
            self.gtia.render_to(&self.antic, self.antic.hpos * 2);
        }
        match addr {
            0x0000..=0x3fff => self.ram[addr as usize] = v,
            0x4000..=0xbfff => self.cart.access(addr),
            0xc000..=0xcfff => self.gtia.write(addr, v),
            0xd400..=0xd4ff => {
                if addr & 0x0f == 0x0a {
                    self.wsync();
                } else {
                    self.antic.write(addr, v);
                }
            }
            0xe800..=0xefff => self.pokey.write(addr, v),
            _ => {}
        }
    }

    /// WSYNC: hold CPU reads from the cycle after next until cycle 105 of
    /// this line, or of the next line when written at 104 or later.
    fn wsync(&mut self) {
        let h = self.antic.hpos as u64;
        let line_start = self.cycles - h;
        self.rdy_until = if h <= 103 { line_start + 105 } else { line_start + CYCLES_PER_LINE as u64 + 105 };
        self.rdy_from = self.cycles + 2;
    }

    /// One machine cycle passes: ANTIC's fetch if it has one, then the chips
    /// advance.
    fn tick(&mut self, dma: Option<DmaFetch>) {
        if let Some(f) = dma {
            let v = if f.slot == crate::antic::Slot::Refresh {
                0xff
            } else {
                self.peek(f.addr).unwrap_or(self.open_bus)
            };
            self.antic.complete(f, v);
            match f.slot {
                crate::antic::Slot::Player(n) => self.gtia.dma_player(n, v),
                crate::antic::Slot::Missile => self.gtia.dma_missiles(v),
                _ => {}
            }
            if let Some(p) = self.probe.as_mut() {
                if f.slot != crate::antic::Slot::Refresh {
                    p.access(&Access {
                        addr: f.addr,
                        value: v,
                        kind: AccessKind::Dma,
                        pc: self.instr_pc,
                        interrupt: false,
                        cycle: self.cycles,
                    });
                }
            }
        }
        let line_start = self.antic.hpos == 0;
        let (p, s) = (self.gtia.pots_powered(), self.gtia.selected_controller());
        self.pokey.tick(line_start, p, s);
        if self.antic.hpos == CYCLES_PER_LINE - 1 {
            self.gtia.render_to(&self.antic, 228);
        }
        self.antic.advance();
        if self.antic.hpos == 0 {
            self.gtia.begin_line();
        }
        self.cycles += 1;
    }

    /// Run cycles ANTIC owns, and cycles a read must wait out, until the CPU
    /// may use the bus.
    fn wait_for_bus(&mut self, is_read: bool) {
        loop {
            if let Some(f) = self.antic.dma() {
                self.tick(Some(f));
                continue;
            }
            if is_read && self.cycles >= self.rdy_from && self.cycles < self.rdy_until {
                self.tick(None);
                continue;
            }
            break;
        }
    }

    fn report(&mut self, addr: u16, value: u8, kind: AccessKind) {
        if let Some(p) = self.probe.as_mut() {
            p.access(&Access {
                addr,
                value,
                kind,
                pc: self.instr_pc,
                interrupt: self.in_interrupt,
                cycle: self.cycles,
            });
        }
    }
}

impl CpuBus for Bus {
    fn read(&mut self, addr: u16) -> u8 {
        self.wait_for_bus(true);
        let v = self.read_mem(addr);
        let kind = if self.first_access && !self.in_interrupt {
            AccessKind::Fetch
        } else {
            AccessKind::Read
        };
        self.first_access = false;
        self.report(addr, v, kind);
        self.tick(None);
        v
    }

    fn write(&mut self, addr: u16, val: u8) {
        self.wait_for_bus(false);
        self.write_mem(addr, val);
        self.first_access = false;
        self.report(addr, val, AccessKind::Write);
        self.tick(None);
    }

    fn nmi(&self) -> bool {
        self.antic.nmi_line
    }

    fn irq(&self) -> bool {
        self.pokey.irq()
    }
}

pub struct Machine {
    pub cpu: Cpu,
    pub bus: Bus,
}

impl Machine {
    /// A machine with this cartridge. With `bios` None it runs the HLE image
    /// (`hle.rs`); a real dump is for differential testing.
    pub fn new(cart: Cart, bios: Option<&[u8]>) -> Self {
        let mut m = Machine { cpu: Cpu::new(), bus: Bus::new(cart, bios) };
        m.reset();
        m
    }

    pub fn reset(&mut self) {
        self.bus.antic.reset();
        self.bus.instr_pc = self.cpu.pc;
        self.bus.in_interrupt = true;
        self.cpu.reset(&mut self.bus);
        self.bus.in_interrupt = false;
    }

    /// Execute one instruction, or one interrupt entry.
    pub fn step(&mut self) {
        self.bus.instr_pc = self.cpu.pc;
        self.bus.first_access = true;
        self.bus.in_interrupt = self.cpu.poll_prev != Pending::None && !self.cpu.halted;
        self.cpu.step(&mut self.bus);
        self.bus.in_interrupt = false;
    }

    /// The last completed frame as RGB, `gtia::WIDTH` x `gtia::HEIGHT`.
    pub fn frame_rgb(&self) -> Vec<u8> {
        let pal = crate::gtia::palette();
        self.bus.gtia.frame.iter().flat_map(|&c| pal[c as usize]).collect()
    }

    /// Run until ANTIC finishes the current frame.
    pub fn run_frame(&mut self) {
        let f = self.bus.antic.frame;
        while self.bus.antic.frame == f {
            self.step();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cart::Mapping;

    /// A cart whose code is `prog` at $8000, entered directly ($BFFD = $FF).
    fn cart_with(prog: &[u8]) -> Cart {
        let mut rom = vec![0xea; 8192];
        rom[..prog.len()].copy_from_slice(prog);
        rom[0x1ffd] = 0xff;
        rom[0x1ffe] = 0x00;
        rom[0x1fff] = 0x80;
        Cart::new(rom, Mapping::Mirror).unwrap()
    }

    /// A stand-in firmware whose reset vector jumps through $BFFE, like the
    /// real one does for a $BFFD=$FF cart.
    fn tiny_bios() -> Vec<u8> {
        let mut b = vec![0u8; 0x800];
        b[0x000..0x003].copy_from_slice(&[0x6c, 0xfe, 0xbf]); // $F800: JMP ($BFFE)
        b[0x7fc] = 0x00;
        b[0x7fd] = 0xf8;
        b
    }

    #[test]
    fn reset_reaches_the_cartridge_through_the_bios_vector() {
        let m = Machine::new(cart_with(&[]), Some(&tiny_bios()));
        assert_eq!(m.cpu.pc, 0xf800);
        let mut m = m;
        m.step();
        assert_eq!(m.cpu.pc, 0x8000);
    }

    #[test]
    fn a_frame_is_262_lines_of_114_cycles() {
        let mut m = Machine::new(cart_with(&[0x4c, 0x00, 0x80]), Some(&tiny_bios()));
        m.run_frame();
        let start = m.bus.cycles;
        m.run_frame();
        let len = m.bus.cycles - start;
        // A frame boundary lands mid-instruction, so allow one instruction.
        assert!((29868 - 3..=29868 + 3).contains(&len), "{len}");
    }

    #[test]
    fn wsync_holds_the_cpu_until_cycle_105() {
        // STA $D40A; then LDA #0 must finish at or after cycle 105.
        let mut m = Machine::new(cart_with(&[0x8d, 0x0a, 0xd4, 0xa9, 0x00, 0x4c, 0x05, 0x80]), Some(&tiny_bios()));
        m.step(); // JMP ($BFFE)
        while m.bus.antic.hpos != 10 {
            m.bus.tick(None);
        }
        m.step(); // STA WSYNC
        m.step(); // LDA #0
        let h = m.bus.antic.hpos;
        assert!((105..=107).contains(&h), "LDA ended at cycle {h}");
    }

    #[test]
    fn refresh_dma_steals_cycles_from_a_busy_loop() {
        // 262 lines x 9 refresh cycles must be missing from the CPU's share.
        let mut m = Machine::new(cart_with(&[0x4c, 0x00, 0x80]), Some(&tiny_bios()));
        m.run_frame();
        let c0 = m.bus.cycles;
        let counter = std::rc::Rc::new(std::cell::RefCell::new(0u64));
        struct Shared(std::rc::Rc<std::cell::RefCell<u64>>);
        impl Probe for Shared {
            fn access(&mut self, a: &Access) {
                if a.kind != AccessKind::Dma {
                    *self.0.borrow_mut() += 1;
                }
            }
        }
        m.bus.probe = Some(Box::new(Shared(counter.clone())));
        m.run_frame();
        let total = m.bus.cycles - c0;
        let cpu = *counter.borrow();
        let stolen = total - cpu;
        assert!((262 * 9 - 9..=262 * 9 + 9).contains(&stolen), "stolen {stolen} of {total}");
    }
}
