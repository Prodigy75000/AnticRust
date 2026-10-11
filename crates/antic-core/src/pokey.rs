// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! POKEY at $E800-$EFFF (16 registers, mirrored).
//!
//! On the 5200 POKEY is the INPUT chip as much as the sound chip: it reads the
//! analog sticks as pot counters and scans the keypads. Both are modelled here
//! because both are objective 2 (an achievement set needs controls a person
//! can work out). Timers run and raise IRQs, and drive the four audio
//! channels (Altirra 5.3-5.5): distortion from the 4, 5 and 9/17-bit noise
//! generators, volume-only mode, the channel 1/2 high-pass flip-flops with
//! their inversion, and the mixer's saturation. Output is resampled to
//! [`SAMPLE_RATE`] and AC-coupled, as the hardware is, so silence is zero.
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

use crate::save::{LoadError, ReadCursor, SaveState, WriteCursor};

/// Audio output rate.
pub const SAMPLE_RATE: u32 = 48_000;

/// The 4-bit and 5-bit noise patterns (Altirra 5.5).
const POLY4: &[u8; 15] = b"000011101100101";
const POLY5: &[u8; 31] = b"0000011100100010101111011010011";

/// Mixer output for a summed volume 0..=60. Nearly linear for one channel,
/// then compressing: two channels at 15 give about 1.5x one channel, all four
/// about 2x (Altirra 5.3, "Volume control"). The curve 30(1 - 2^(-x/15))
/// meets those points; it is a fit, not a measurement.
fn mix_level(sum: u8) -> f32 {
    30.0 * (1.0 - (2.0f32).powf(-(sum as f32) / 15.0))
}

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
    /// Keypad scan counter, 0..63, one step a scan line. Its value is the
    /// keyboard code being tested: K0-K5.
    kb_div: u8,
    pub controllers: [Controller; 4],
    // ---- audio ----
    /// Cycle position in the 4-bit and 5-bit noise patterns.
    p4: u8,
    p5: u8,
    /// Output flip-flop per channel.
    ff: [bool; 4],
    /// High-pass flip-flops for channels 1 and 2.
    hp: [bool; 2],
    /// Resampler: summed level over the cycles of the current sample.
    acc: f32,
    acc_cycles: u32,
    frac: u32,
    /// DC blocker state.
    dc_prev_in: f32,
    dc_prev_out: f32,
    /// Samples produced since the last [`Pokey::take_samples`].
    pub samples: Vec<i16>,
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
            p4: 0,
            p5: 0,
            ff: [false; 4],
            hp: [false; 2],
            acc: 0.0,
            acc_cycles: 0,
            frac: 0,
            dc_prev_in: 0.0,
            dc_prev_out: 0.0,
            samples: Vec::new(),
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
                // STIMER: all channels restart from their periods, and the
                // output flip-flops go to 1.
                for ch in 0..4 {
                    self.counter[ch] = self.period(ch);
                }
                self.ff = [true; 4];
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
        // Polynomial counters run every cycle unless SKCTL holds init, which
        // also holds the 64 kHz and 15 kHz clocks.
        if self.skctl & 3 != 0 {
            let p = self.poly17;
            self.poly17 = (p >> 1) | ((((p << 16) ^ (p << 11)) & 0x10000) & 0x1ffff);
            let q = self.poly9;
            self.poly9 = (q >> 1) | (((q << 8) ^ (q << 3)) & 0x100);
            self.p4 = (self.p4 + 1) % 15;
            self.p5 = (self.p5 + 1) % 31;
        } else {
            self.div64 = 0;
            self.div15 = 0;
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
            if fire1 && !link12 {
                self.raise(IRQ_T1);
            }
            if fire2 {
                self.raise(IRQ_T2);
            }
            if fire4 {
                self.raise(IRQ_T4);
            }
            for (ch, f) in [fire1, fire2, fire3, fire4].into_iter().enumerate() {
                if f {
                    self.clock_output(ch);
                }
            }
            // High-pass: channel 3 clocks channel 1's flip-flop, 4 clocks 2's.
            if fire3 {
                self.hp[0] = self.ff[0];
            }
            if fire4 {
                self.hp[1] = self.ff[1];
            }
        }
        self.mix();

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

        // Keypad. POKEY scans 64 keyboard codes, one a line. The 5200 wires
        // only K1-K4 to the keypad; K0 and K5 are not connected, so a held
        // key matches at several scan addresses, and with K0 floating the
        // code that sticks has bit 0 set. The result is two key events
        // every 64 lines, bit 0 always 1 and bit 5 alternating: Start reads
        // $19 then $39. a5200's input.c describes the same behaviour.
        // Games that test KBCODE whole depend on it. Pitfall! polls for $19
        // (Start) and $11 (Pause). Popeye acts only on the FIRST key
        // interrupt of a press, masks to 5 bits, and wants $19, so an
        // even code first means Start is lost.
        if line_start {
            self.kb_div = (self.kb_div + 1) & 63;
            if self.skctl & 2 != 0 {
                let c = self.controllers[selected];
                if let Some(k) = c.key {
                    if self.kb_div & 1 == 1 && (self.kb_div >> 1) & 0x0f == k.code() {
                        self.kbcode = self.kb_div | if c.top { 0xc0 } else { 0 };
                        self.raise(IRQ_KEY);
                    }
                }
                if c.top && self.kb_div & 31 == 0 {
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

    /// A timer pulse reaches channel `ch`'s output stage. Noise reaches the
    /// channels one cycle apart, channel 1 first.
    fn clock_output(&mut self, ch: usize) {
        let c = self.audc[ch];
        let at = |pattern: &[u8], pos: u8| pattern[(pos as usize + pattern.len() - ch) % pattern.len()] == b'1';
        // Bit 7 clear: the 5-bit generator masks out some pulses.
        if c & 0x80 == 0 && !at(POLY5, self.p5) {
            return;
        }
        self.ff[ch] = if c & 0x20 != 0 {
            !self.ff[ch]
        } else if c & 0x40 != 0 {
            at(POLY4, self.p4)
        } else if self.audctl & 0x80 != 0 {
            self.poly9 >> ch & 1 != 0
        } else {
            self.poly17 >> ch & 1 != 0
        };
    }

    /// Sum the channels for this cycle and feed the resampler.
    fn mix(&mut self) {
        let mut sum = 0u8;
        for ch in 0..4 {
            let c = self.audc[ch];
            let vol = c & 0x0f;
            if vol == 0 {
                continue;
            }
            let on = if c & 0x10 != 0 {
                true // volume-only: forced on after the high-pass logic
            } else if ch < 2 {
                // XOR with the high-pass flip-flop, which is forced to 1 when
                // the filter is off: channels 1 and 2 come out inverted.
                let hp_on = self.audctl & (0x04 >> ch) != 0;
                self.ff[ch] ^ if hp_on { self.hp[ch] } else { true }
            } else {
                self.ff[ch]
            };
            if on {
                sum += vol;
            }
        }
        self.acc += mix_level(sum);
        self.acc_cycles += 1;
        self.frac += SAMPLE_RATE;
        if self.frac >= crate::CPU_HZ {
            self.frac -= crate::CPU_HZ;
            let level = self.acc / self.acc_cycles as f32;
            self.acc = 0.0;
            self.acc_cycles = 0;
            // AC coupling: a one-pole high-pass near 20 Hz.
            let out = level - self.dc_prev_in + 0.9974 * self.dc_prev_out;
            self.dc_prev_in = level;
            self.dc_prev_out = out;
            // Full scale is the mixer's ceiling of 30.
            let s = (out / 30.0 * 32767.0).clamp(-32768.0, 32767.0);
            self.samples.push(s as i16);
        }
    }

    /// Drain the audio produced so far.
    pub fn take_samples(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.samples)
    }

    fn raise(&mut self, bit: u8) {
        if self.irqen & bit != 0 {
            self.irq_pending |= bit;
        }
    }
}

/// The controllers are input, set by the frontend every frame, and the
/// resampler and DC blocker are the audio OUTPUT path (floats, which the
/// save-state contract keeps out of state), so none of those is saved. A
/// load restarts the output path from silence.
impl SaveState for Pokey {
    fn save(&self, w: &mut WriteCursor) {
        w.bytes(&self.audf);
        w.bytes(&self.audc);
        for v in [self.audctl, self.skctl, self.irqen, self.irq_pending, self.kbcode] {
            w.u8(v);
        }
        for c in self.counter {
            w.u16(c);
        }
        w.u8(self.div64);
        w.u8(self.div15);
        w.u32(self.poly17);
        w.u16(self.poly9);
        w.u8(self.pot_count);
        w.bytes(&self.pot_latch);
        for v in [self.allpot, self.pot_line_div, self.kb_div, self.p4, self.p5] {
            w.u8(v);
        }
        for v in self.ff {
            w.bool(v);
        }
        for v in self.hp {
            w.bool(v);
        }
    }

    fn load(&mut self, r: &mut ReadCursor) -> Result<(), LoadError> {
        r.bytes(&mut self.audf)?;
        r.bytes(&mut self.audc)?;
        self.audctl = r.u8()?;
        self.skctl = r.u8()?;
        self.irqen = r.u8()?;
        self.irq_pending = r.u8()?;
        self.kbcode = r.u8()?;
        for c in self.counter.iter_mut() {
            *c = r.u16()?;
        }
        self.div64 = r.u8()?;
        self.div15 = r.u8()?;
        self.poly17 = r.u32()?;
        self.poly9 = r.u16()?;
        self.pot_count = r.u8()?;
        r.bytes(&mut self.pot_latch)?;
        self.allpot = r.u8()?;
        self.pot_line_div = r.u8()?;
        self.kb_div = r.u8()?;
        self.p4 = r.u8()?;
        self.p5 = r.u8()?;
        if self.p4 >= 15 || self.p5 >= 31 || self.poly17 > 0x1ffff || self.poly9 > 0x1ff {
            return Err(LoadError::BadValue("POKEY noise generator"));
        }
        for v in self.ff.iter_mut() {
            *v = r.bool()?;
        }
        for v in self.hp.iter_mut() {
            *v = r.bool()?;
        }
        self.acc = 0.0;
        self.acc_cycles = 0;
        self.frac = 0;
        self.dc_prev_in = 0.0;
        self.dc_prev_out = 0.0;
        self.samples.clear();
        Ok(())
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

    /// Frequency of a sample stream by counting rising zero crossings.
    fn pitch(s: &[i16]) -> f64 {
        let ups: Vec<usize> = (1..s.len()).filter(|&i| s[i - 1] < 0 && s[i] >= 0).collect();
        let span = (ups[ups.len() - 1] - ups[0]) as f64;
        (ups.len() - 1) as f64 * SAMPLE_RATE as f64 / span
    }

    fn run(p: &mut Pokey, cycles: u32) -> Vec<i16> {
        for _ in 0..cycles {
            p.tick(false, true, 0);
        }
        p.take_samples()
    }

    #[test]
    fn audf_72_pure_tone_on_64khz_plays_437_8_hz() {
        // Altirra 5.3: AUDF=72 on the 64 kHz clock pulses at 875.6 Hz and
        // the pure-tone flip-flop halves that to 437.8 Hz.
        let mut p = Pokey::new();
        p.skctl = 3;
        p.write(0x06, 72); // AUDF4
        p.write(0x07, 0xa8); // AUDC4: pure tone, volume 8
        let s = run(&mut p, crate::CPU_HZ);
        let f = pitch(&s[4800..]);
        assert!((f - 437.8).abs() < 1.0, "measured {f:.2} Hz");
    }

    #[test]
    fn silence_is_zero_even_with_a_constant_channel() {
        // Volume-only at a fixed level is a DC step; the AC coupling must
        // bring it back to zero, or the stream ends on a click.
        let mut p = Pokey::new();
        p.skctl = 3;
        p.write(0x07, 0x1f);
        let s = run(&mut p, crate::CPU_HZ * 2);
        let tail = &s[s.len() - 480..];
        assert!(tail.iter().all(|&v| v.abs() < 64), "tail {:?}", &tail[..8]);
        assert!(s[..480].iter().any(|&v| v.abs() > 10_000), "the step itself was audible");
    }

    /// The output flip-flop of channel `ch` sampled after each of its timer
    /// pulses, for `n` pulses.
    fn ff_per_pulse(p: &mut Pokey, ch: usize, n: usize) -> Vec<bool> {
        let mut seen = Vec::new();
        while seen.len() < n {
            let before = p.counter[ch];
            p.tick(false, true, 0);
            if p.counter[ch] > before {
                seen.push(p.ff[ch]);
            }
        }
        seen
    }

    #[test]
    fn pure_tone_alternates_on_every_pulse_and_noise_does_not() {
        let mut p = Pokey::new();
        p.skctl = 3;
        p.write(0x04, 0x05); // AUDF3
        p.write(0x05, 0xa8); // AUDC3: pure tone
        let tone = ff_per_pulse(&mut p, 2, 64);
        assert!(tone.windows(2).all(|w| w[0] != w[1]), "pure tone must toggle every pulse");

        p.write(0x05, 0x88); // AUDC3: poly-17 noise, no poly-5 mask
        let noise = ff_per_pulse(&mut p, 2, 256);
        let repeats = noise.windows(2).filter(|w| w[0] == w[1]).count();
        assert!(repeats > 64, "noise repeated a level only {repeats} times in 255");
        assert!(noise.iter().any(|&b| b) && noise.iter().any(|&b| !b));
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
        for _ in 0..64 {
            p.tick(true, true, 1);
        }
        assert_eq!(p.kbcode & 0x1e, 0b1100 << 1);
        assert!(p.irq());
    }

    #[test]
    fn a_held_key_reads_with_bit_0_set_and_bit_5_alternating() {
        // Pitfall! starts on KBCODE == $19 exactly; a model that reports
        // only code << 1 never shows it, and the game never starts.
        let mut p = Pokey::new();
        p.skctl = 2;
        p.controllers[0].key = Some(Key::Start);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..128 {
            p.tick(true, true, 0);
            seen.insert(p.kbcode);
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), vec![0x19, 0x39, 0xff]);
        p.controllers[0].key = Some(Key::Pause);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..128 {
            p.tick(true, true, 0);
            seen.insert(p.kbcode);
        }
        assert!(seen.contains(&0x11), "Pause reads $11 at some point: {seen:x?}");
    }
}
