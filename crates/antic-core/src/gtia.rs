// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! GTIA at $C000-$CFFF, 32 registers mirrored through the block.
//!
//! For now a register file plus the inputs the 5200 wires to it: the bottom
//! fire buttons on TRIG0-3, and CONSOL as an OUTPUT selecting which
//! controller's keypad POKEY scans (bits 0-1) and powering the analog sticks
//! (bit 2). Rendering and collisions come with the renderer; collisions read
//! zero until then, which is the "nothing touched" answer rather than a hang.

#[derive(Clone, Debug)]
pub struct Gtia {
    /// Last value written to each register.
    pub regs: [u8; 32],
    /// Bottom button per controller, true while held.
    pub trigger: [bool; 4],
    /// Latched trigger state (GRACTL bit 2).
    latched: [bool; 4],
}

impl Default for Gtia {
    fn default() -> Self {
        Gtia { regs: [0; 32], trigger: [false; 4], latched: [false; 4] }
    }
}

pub const GRACTL: usize = 0x1d;
pub const CONSOL: usize = 0x1f;

impl Gtia {
    pub fn new() -> Self {
        Self::default()
    }

    /// The controller POKEY's keypad scan currently sees.
    pub fn selected_controller(&self) -> usize {
        (self.regs[CONSOL] & 3) as usize
    }

    /// CONSOL bit 2 powers the sticks; without it POTs read 228.
    pub fn pots_powered(&self) -> bool {
        self.regs[CONSOL] & 4 != 0
    }

    pub fn read(&mut self, addr: u16) -> u8 {
        let r = (addr & 0x1f) as usize;
        match r {
            0x00..=0x0f => 0,
            0x10..=0x13 => {
                let n = r - 0x10;
                let held = self.trigger[n] || (self.regs[GRACTL] & 4 != 0 && self.latched[n]);
                if self.trigger[n] {
                    self.latched[n] = true;
                }
                (!held) as u8
            }
            0x14 => 0x0f, // NTSC
            0x1f => !self.regs[CONSOL] & 0x0f,
            _ => 0x0f,
        }
    }

    pub fn write(&mut self, addr: u16, v: u8) {
        let r = (addr & 0x1f) as usize;
        self.regs[r] = v;
        if r == GRACTL && v & 4 == 0 {
            self.latched = [false; 4];
        }
    }
}
