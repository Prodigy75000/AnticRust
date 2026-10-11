// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! POKEY at $E800-$EFFF (16 registers, mirrored).
//!
//! On the 5200 POKEY is the INPUT chip as much as the sound chip: it reads the
//! analog sticks as pot counters and scans the keypads. Both are modelled here
//! because both are objective 2 (an achievement set needs controls a person
//! can work out). Timers run and raise IRQs; audio output is not generated
//! yet.
//!
//! **The sticks are analog and do not self-centre.** A pot reads the count
//! POKEY reached when the stick's capacitor charged, 1..228. There is no
//! "centre" input, only a position, and the frontend decides what a digital
//! pad maps to. See Altirra Hardware Reference Manual 5.9 and 13.3.

/// Keypad scan codes (KBCODE bits 1-4), from Altirra 13.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Digit(u8),
    Star,
    Hash,
    Start,
    Pause,
    Reset,
}

impl Key {
    pub fn code(self) -> u8 {
        match self {
            Key::Digit(0) => 0b0010,
            Key::Digit(1) => 0b1111,
            Key::Digit(2) => 0b1110,
            Key::Digit(3) => 0b1101,
            Key::Digit(4) => 0b1011,
            Key::Digit(5) => 0b1010,
            Key::Digit(6) => 0b1001,
            Key::Digit(7) => 0b0111,
            Key::Digit(8) => 0b0110,
            Key::Digit(9) => 0b0101,
            Key::Digit(_) => 0b0010,
            Key::Star => 0b0011,
            Key::Hash => 0b0001,
            Key::Start => 0b1100,
            Key::Pause => 0b1000,
            Key::Reset => 0b0100,
        }
    }
}

/// One controller as POKEY sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controller {
    /// Stick position as pot counts, x then y. Lower is left/up.
    pub pot: [u8; 2],
    pub key: Option<Key>,
    /// The top button, wired to the keyboard's KR2 line.
    pub top: bool,
}

impl Default for Controller {
    fn default() -> Self {
        Controller { pot: [POT_CENTRE; 2], key: None, top: false }
    }
}

/// Pot count reached at full travel.
pub const POT_MAX: u8 = 228;
/// The midpoint of the pot range. Hardware has no centre detent; this is
/// what a frontend reports for a stick at rest.
pub const POT_CENTRE: u8 = 114;

pub const IRQ_T1: u8 = 0x01;
pub const IRQ_T2: u8 = 0x02;
pub const IRQ_T4: u8 = 0x04;
pub const IRQ_KEY: u8 = 0x40;
pub const IRQ_BREAK: u8 = 0x80;

#[derive(Clone, Debug)]
pub struct Pokey {
    pub audf: [u8; 4],
    pub audc: [u8; 4],
    pub audctl: u8,
    pub skctl: u8,
    pub irqen: u8,
    /// Pending IRQ sources, active high (IRQST reads the inverse).
    pub irq_pending: u8,
    pub kbcode: u8,
    /// Channel down-counters.
    counter: [u16; 4],
    /// Cycle counters for the 64 kHz and 15 kHz base clocks.
    div64: u8,
    div15: u8,
    /// 17-bit and 9-bit polynomial counters.
    poly17: u32,
    poly9: u16,
    /// Pot scan.
    pot_count: u8,
    pot_latch: [u8; 8],
    allpot: u8,
    pot_line_div: u8,
    /// Keypad scan, in scan lines.
    kb_div: u8,
    pub controllers: [Controller; 4],
}

impl Default for Pokey {
    fn default() -> Self {
        Pokey {
            audf: [0; 4],
            audc: [0; 4],
            audctl: 0,
            skctl: 0,
            irqen: 0,
            irq_pending: 0,
            kbcode: 0xff,
            counter: [0; 4],
            div64: 0,
            div15: 0,
            poly17: 0x1ffff,
            poly9: 0x1ff,
            pot_count: 0,
            pot_latch: [0; 8],
            allpot: 0xff,
            pot_line_div: 0,
            kb_div: 0,
            controllers: [Controller::default(); 4],
        }
    }
}

impl Pokey {
    pub fn new() -> Self {
        Self::default()
    }

    /// The IRQ line: any pending source that is enabled.
    pub fn irq(&self) -> bool {
        self.irq_pending & self.irqen != 0
    }

    /// RANDOM: the low byte of the 9-bit or 17-bit shift register in its
    /// right-shifting form (Altirra appendix A). The 9-bit sequence is checked
    /// against the manual's table in the tests; the 17-bit byte choice is not
    /// yet verified against hardware.
    pub fn random(&self) -> u8 {
        if self.audctl & 0x80 != 0 {
            self.poly9 as u8
        } else {
            self.poly17 as u8
        }
    }

    pub fn read(&mut self, addr: u16, pots_powered: bool, selected: usize) -> u8 {
        match addr & 0x0f {
            r @ 0x00..=0x07 => {
                let _ = pots_powered;
                if self.allpot & (1 << r) == 0 {
                    self.pot_latch[r as usize]
                } else {
                    self.pot_count
                }
            }
            0x08 => self.allpot,
            0x09 => self.kbcode,
            0x0a => self.random(),
            0x0d => 0xff, // SERIN: nothing on the SIO port
            0x0e => !self.irq_pending | 0x08,
            0x0f => {
                let c = self.controllers[selected];
                let mut s = 0xffu8;
                if c.key.is_some() && self.skctl & 2 != 0 {
                    s &= !0x04;
                }
                if c.top {
                    s &= !0x08;
                }
                s
            }
            _ => 0xff,
        }
    }

    pub fn write(&mut self, addr: u16, v: u8) {
        match addr & 0x0f {
            r @ (0x00 | 0x02 | 0x04 | 0x06) => self.audf[(r >> 1) as usize] = v,
            r @ (0x01 | 0x03 | 0x05 | 0x07) => self.audc[(r >> 1) as usize] = v,
            0x08 => self.audctl = v,
            0x09 => {
                // STIMER: all channels restart from their periods.
                for ch in 0..4 {
                    self.counter[ch] = self.period(ch);
                }
            }
            0x0b => {
                // POTGO
                self.pot_count = 0;
                self.allpot = 0xff;
                self.pot_latch = [0; 8];
            }
            0x0e => {
                self.irqen = v;
                self.irq_pending &= v;
            }
            0x0f => self.skctl = v,
            _ => {}
        }
    }

    /// Down-counter reload for a channel, in its own clock's ticks.
    fn period(&self, ch: usize) -> u16 {
        let fast = (ch == 0 && self.audctl & 0x40 != 0) || (ch == 2 && self.audctl & 0x20 != 0);
        let linked = (ch == 1 && self.audctl & 0x10 != 0) || (ch == 3 && self.audctl & 0x08 != 0);
        if linked {
            let lo_fast = (ch == 1 && self.audctl & 0x40 != 0) || (ch == 3 && self.audctl & 0x20 != 0);
            let v = ((self.audf[ch] as u16) << 8) | self.audf[ch - 1] as u16;
            v + if lo_fast { 6 } else { 0 }
        } else {
            self.audf[ch] as u16 + if fast { 3 } else { 0 }
        }
    }

    /// One machine cycle. `line_start` marks cycle 0 of a scan line;
    /// `pots_powered` is GTIA CONSOL bit 2, `selected` its bits 0-1.
    pub fn tick(&mut self, line_start: bool, pots_powered: bool, selected: usize) {
        // Polynomial counters run every cycle unless SKCTL holds init.
        if self.skctl & 3 != 0 {
            let p = self.poly17;
            self.poly17 = (p >> 1) | ((((p << 16) ^ (p << 11)) & 0x10000) & 0x1ffff);
            let q = self.poly9;
            self.poly9 = (q >> 1) | (((q << 8) ^ (q << 3)) & 0x100);
        }

        // Base clocks.
        self.div64 += 1;
        let tick64 = self.div64 >= 28;
        if tick64 {
            self.div64 = 0;
        }
        self.div15 += 1;
        let tick15 = self.div15 >= 114;
        if tick15 {
            self.div15 = 0;
        }
        let slow = if self.audctl & 1 != 0 { tick15 } else { tick64 };

        if self.skctl & 3 != 0 {
            let fast1 = self.audctl & 0x40 != 0;
            let fast3 = self.audctl & 0x20 != 0;
            let link12 = self.audctl & 0x10 != 0;
            let link34 = self.audctl & 0x08 != 0;
            let fire1 = self.clock(0, if fast1 { true } else { slow }, !link12);
            let fire2 = if link12 { self.clock(1, if fast1 { true } else { slow }, true) } else { self.clock(1, slow, true) };
            let fire3 = self.clock(2, if fast3 { true } else { slow }, !link34);
            let fire4 = if link34 { self.clock(3, if fast3 { true } else { slow }, true) } else { self.clock(3, slow, true) };
            let _ = fire3;
            if fire1 && !link12 {
                self.raise(IRQ_T1);
            }
            if fire2 {
                self.raise(IRQ_T2);
            }
            if fire4 {
                self.raise(IRQ_T4);
            }
        }

        // Pot scan: slow mode counts once per scan line, fast mode per cycle.
        let pot_tick = if self.skctl & 4 != 0 {
            true
        } else if line_start {
            self.pot_line_div = 0;
            true
        } else {
            false
        };
        if pot_tick && self.allpot != 0 {
            if self.pot_count < POT_MAX {
                self.pot_count += 1;
            }
            for p in 0..8 {
                if self.allpot & (1 << p) == 0 {
                    continue;
                }
                let target = if pots_powered { self.controllers[p / 2].pot[p % 2] } else { POT_MAX };
                if self.pot_count >= target || self.pot_count == POT_MAX {
                    self.pot_latch[p] = self.pot_count;
                    self.allpot &= !(1 << p);
                }
            }
        }

        // Keypad: with debounce off, a held key is reported every 32 lines.
        if line_start {
            self.kb_div = (self.kb_div + 1) & 31;
            if self.kb_div == 0 && self.skctl & 2 != 0 {
                let c = self.controllers[selected];
                if let Some(k) = c.key {
                    self.kbcode = (k.code() << 1) | if c.top { 0xc0 } else { 0 };
                    self.raise(IRQ_KEY);
                }
                if c.top {
                    self.raise(IRQ_BREAK);
                }
            }
        }
    }

    /// Count a channel down on its clock; true when it underflows. The
    /// linked high channel of a pair is clocked by its partner's underflow,
    /// which `period` folds into a 16-bit count instead.
    fn clock(&mut self, ch: usize, clock: bool, enabled: bool) -> bool {
        if !clock || !enabled {
            return false;
        }
        if self.counter[ch] == 0 {
            self.counter[ch] = self.period(ch);
            true
        } else {
            self.counter[ch] -= 1;
            false
        }
    }

    fn raise(&mut self, bit: u8) {
        if self.irqen & bit != 0 {
            self.irq_pending |= bit;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_bit_random_matches_the_manuals_published_sequence() {
        // Altirra 14.4, RANDOM: from the $00 state the 6502 sees
        // 00 80 40 20 10 88 44 22 11 88 C4 62 31 98.
        let mut p = Pokey::new();
        p.skctl = 3;
        p.audctl = 0x80;
        p.poly9 = 0x100;
        let mut seen = Vec::new();
        for _ in 0..14 {
            seen.push(p.random());
            p.tick(false, true, 0);
        }
        assert_eq!(
            seen,
            vec![0x00, 0x80, 0x40, 0x20, 0x10, 0x88, 0x44, 0x22, 0x11, 0x88, 0xc4, 0x62, 0x31, 0x98]
        );
    }

    #[test]
    fn a_pot_latches_its_position_and_clears_its_allpot_bit() {
        let mut p = Pokey::new();
        p.controllers[0].pot = [40, 200];
        p.write(0x0b, 0);
        for line in 0..230 {
            p.tick(true, true, 0);
            if line == 50 {
                assert_eq!(p.read(0x08, true, 0) & 0x03, 0x02, "x done, y still counting");
            }
        }
        assert_eq!(p.read(0x00, true, 0), 40);
        assert_eq!(p.read(0x01, true, 0), 200);
        assert_eq!(p.read(0x08, true, 0) & 0x03, 0);
    }

    #[test]
    fn unpowered_sticks_read_full_scale() {
        let mut p = Pokey::new();
        p.controllers[0].pot = [40, 40];
        p.write(0x0b, 0);
        for _ in 0..230 {
            p.tick(true, false, 0);
        }
        assert_eq!(p.read(0x00, false, 0), POT_MAX);
    }

    #[test]
    fn timer_1_raises_its_irq_after_its_period() {
        // AUDF1=9 on the 64 kHz clock: 10 ticks of 28 cycles.
        let mut p = Pokey::new();
        p.skctl = 3;
        p.write(0x00, 9);
        p.write(0x0e, IRQ_T1);
        p.write(0x09, 0);
        let mut fired_at = None;
        for c in 0..2000u32 {
            p.tick(false, true, 0);
            if p.irq() {
                fired_at = Some(c);
                break;
            }
        }
        let c = fired_at.expect("timer 1 never fired");
        assert!((270..=310).contains(&c), "fired at cycle {c}");
    }

    #[test]
    fn disabling_an_irq_in_irqen_clears_it() {
        let mut p = Pokey::new();
        p.irqen = IRQ_KEY;
        p.irq_pending = IRQ_KEY;
        assert!(p.irq());
        assert_eq!(p.read(0x0e, true, 0) & IRQ_KEY, 0);
        p.write(0x0e, 0);
        assert!(!p.irq());
        assert_eq!(p.read(0x0e, true, 0) & IRQ_KEY, IRQ_KEY);
    }

    #[test]
    fn keypad_start_is_reported_as_its_scan_code() {
        let mut p = Pokey::new();
        p.skctl = 2;
        p.irqen = IRQ_KEY;
        p.controllers[1].key = Some(Key::Start);
        for _ in 0..32 {
            p.tick(true, true, 1);
        }
        assert_eq!(p.kbcode & 0x1e, 0b1100 << 1);
        assert!(p.irq());
    }
}
