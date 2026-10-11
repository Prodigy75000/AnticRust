// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! GTIA at $C000-$CFFF, 32 registers mirrored through the block, and the
//! picture it makes.
//!
//! The 5200 wires two inputs to GTIA: the bottom fire buttons on TRIG0-3, and
//! CONSOL as an OUTPUT that selects which controller's keypad POKEY scans
//! (bits 0-1) and powers the analog sticks (bit 2).
//!
//! Rendering is per color clock, 228 to a line, two hi-res pixels each. The
//! bus renders up to the current cycle before any register write that can
//! change the picture, so mid-line colour changes (the reason DLIs exist)
//! land where the hardware puts them. ANTIC supplies the playfield; GTIA
//! adds players and missiles, resolves priority, records collisions, and
//! outputs a colour byte per pixel. The palette turns colour bytes into RGB.
//!
//! Written from the Altirra Hardware Reference Manual, chapter 6 and
//! appendix D. The playfield pipeline delay between fetch and display is not
//! modelled; fetched bytes appear at their nominal positions.

use crate::antic::Antic;

/// First and one-past-last color clock kept in the frame: the wide playfield.
pub const FIRST_CC: u16 = 0x20;
pub const LAST_CC: u16 = 0xe0;
/// Frame size in hi-res pixels.
pub const WIDTH: usize = ((LAST_CC - FIRST_CC) * 2) as usize;
pub const HEIGHT: usize = 240;
/// First scan line kept in the frame.
pub const FIRST_LINE: u16 = 8;

pub const GRACTL: usize = 0x1d;
pub const CONSOL: usize = 0x1f;
const PRIOR: usize = 0x1b;
const COLPM0: usize = 0x12;
const COLPF0: usize = 0x16;
const COLBK: usize = 0x1a;

/// What the playfield shows at one hi-res pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pf {
    Bak,
    /// PF0-PF3.
    Color(u8),
    /// Hi-res: PF2 hue, with PF1's luminance when lit.
    Hires(bool),
    /// GTIA mode nibble (PRIOR bits 6-7 set).
    Gtia(u8),
}

#[derive(Clone)]
pub struct Gtia {
    /// Last value written to each register.
    pub regs: [u8; 32],
    /// Collision latches: missile-playfield, player-playfield, missile-player,
    /// player-player, four of each, in register order $00-$0F.
    pub collisions: [u8; 16],
    /// Bottom button per controller, true while held.
    pub trigger: [bool; 4],
    latched: [bool; 4],
    /// The frame, one colour byte per hi-res pixel.
    pub frame: Vec<u8>,
    /// Next color clock to render on the current line.
    cursor: u16,
}

impl Default for Gtia {
    fn default() -> Self {
        Gtia {
            regs: [0; 32],
            collisions: [0; 16],
            trigger: [false; 4],
            latched: [false; 4],
            frame: vec![0; WIDTH * HEIGHT],
            cursor: 0,
        }
    }
}

impl std::fmt::Debug for Gtia {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gtia").field("regs", &self.regs).finish()
    }
}

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
            0x00..=0x0f => self.collisions[r],
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
        match r {
            GRACTL if v & 4 == 0 => self.latched = [false; 4],
            0x1e => self.collisions = [0; 16], // HITCLR
            _ => {}
        }
    }

    /// Player/missile DMA delivers a byte; GRACTL decides whether it lands.
    pub fn dma_player(&mut self, n: u8, v: u8) {
        if self.regs[GRACTL] & 2 != 0 {
            self.regs[0x0d + n as usize] = v;
        }
    }

    pub fn dma_missiles(&mut self, v: u8) {
        if self.regs[GRACTL] & 1 != 0 {
            self.regs[0x11] = v;
        }
    }

    /// Start a new line: nothing rendered yet.
    pub fn begin_line(&mut self) {
        self.cursor = 0;
    }

    /// Render color clocks from the cursor up to (not including) `to_cc` of
    /// the line ANTIC is on.
    pub fn render_to(&mut self, antic: &Antic, to_cc: u16) {
        let line = antic.vline;
        let to = to_cc.min(228);
        if self.cursor >= to {
            return;
        }
        let from = self.cursor;
        self.cursor = to;
        if !(FIRST_LINE..FIRST_LINE + HEIGHT as u16).contains(&line) {
            return;
        }
        let row = (line - FIRST_LINE) as usize * WIDTH;
        let active = antic_active(antic);
        for cc in from.max(FIRST_CC)..to.min(LAST_CC) {
            let (l, r) = if active { playfield(antic, self.regs[PRIOR], cc) } else { (Pf::Bak, Pf::Bak) };
            let (cl, cr) = self.compose(cc, l, r);
            let x = row + ((cc - FIRST_CC) * 2) as usize;
            self.frame[x] = cl;
            self.frame[x + 1] = cr;
        }
    }

    /// Players and missiles covering color clock `cc`: bit n for object n.
    fn objects(&self, cc: u16) -> (u8, u8) {
        let mut players = 0u8;
        let mut missiles = 0u8;
        for n in 0..4 {
            let pos = self.regs[n] as u16;
            let w = match self.regs[0x08 + n] & 3 {
                1 => 2,
                3 => 4,
                _ => 1,
            };
            if cc >= pos && cc < pos + 8 * w {
                let bit = 7 - (cc - pos) / w;
                if self.regs[0x0d + n] >> bit & 1 != 0 {
                    players |= 1 << n;
                }
            }
            let mpos = self.regs[0x04 + n] as u16;
            let mw = match (self.regs[0x0c] >> (2 * n)) & 3 {
                1 => 2,
                3 => 4,
                _ => 1,
            };
            if cc >= mpos && cc < mpos + 2 * mw {
                let bit = 2 * n as u16 + 1 - (cc - mpos) / mw;
                if self.regs[0x11] >> bit & 1 != 0 {
                    missiles |= 1 << n;
                }
            }
        }
        (players, missiles)
    }

    /// Resolve one color clock (two hi-res halves of playfield) to colours,
    /// and latch collisions.
    fn compose(&mut self, cc: u16, l: Pf, r: Pf) -> (u8, u8) {
        let prior = self.regs[PRIOR];
        let (players, missiles) = self.objects(cc);
        // Collisions use the playfield colour of the color clock; a lit
        // hi-res pixel counts as PF2.
        let pf_bits = |p: Pf| match p {
            Pf::Color(k) => 1u8 << k,
            Pf::Hires(true) => 1 << 2,
            _ => 0,
        };
        let pfc = pf_bits(l) | pf_bits(r);
        for n in 0..4 {
            if missiles & (1 << n) != 0 {
                self.collisions[n] |= pfc;
                self.collisions[0x08 + n] |= players;
            }
            if players & (1 << n) != 0 {
                self.collisions[0x04 + n] |= pfc;
                self.collisions[0x0c + n] |= players & !(1 << n);
            }
        }
        // Fifth player: missiles take PF3's colour and priority.
        let fifth = prior & 0x10 != 0;
        let mut objs = players;
        if !fifth {
            objs |= missiles;
        }
        (self.pixel(l, objs, fifth && missiles != 0), self.pixel(r, objs, fifth && missiles != 0))
    }

    fn pixel(&self, pf: Pf, objs: u8, fifth: bool) -> u8 {
        let prior = self.regs[PRIOR];
        let pm_color = |n: usize| self.regs[COLPM0 + n];
        // Playfield class and colour.
        let (class, pf_color) = match pf {
            Pf::Bak => (None, self.regs[COLBK]),
            Pf::Color(k) => (Some(k), self.regs[COLPF0 + k as usize]),
            Pf::Hires(lit) => {
                let c = if lit {
                    (self.regs[COLPF0 + 2] & 0xf0) | (self.regs[COLPF0 + 1] & 0x0e)
                } else {
                    self.regs[COLPF0 + 2]
                };
                (Some(2), c)
            }
            Pf::Gtia(nib) => (None, self.gtia_color(nib)),
        };
        let (class, pf_color) = if fifth && class.map_or(true, |k| k < 3) {
            (Some(3), self.regs[COLPF0 + 3])
        } else {
            (class, pf_color)
        };
        if objs == 0 {
            return pf_color;
        }
        // Player colour, with multicolor OR of pairs.
        let multi = prior & 0x20 != 0;
        let first = objs.trailing_zeros() as usize;
        let mut pc = pm_color(first);
        if multi {
            if first < 2 && objs & 3 == 3 {
                pc = pm_color(0) | pm_color(1);
            } else if first >= 2 && objs & 0xc == 0xc {
                pc = pm_color(2) | pm_color(3);
            }
        }
        let Some(k) = class else { return pc };
        let low_players = objs & 3 != 0;
        let pri = prior & 0x0f;
        let player_wins = if pri & 0x04 != 0 {
            false
        } else if pri & 0x08 != 0 {
            k >= 2
        } else if pri & 0x02 != 0 {
            low_players
        } else {
            true
        };
        if player_wins {
            pc
        } else {
            pf_color
        }
    }

    fn gtia_color(&self, nib: u8) -> u8 {
        let bk = self.regs[COLBK];
        match self.regs[PRIOR] >> 6 {
            1 => (bk & 0xf0) | nib,
            2 => {
                const MAP: [usize; 9] = [0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a];
                self.regs[*MAP.get(nib as usize).unwrap_or(&0x1a)]
            }
            _ => (nib << 4) | (bk & 0x0f),
        }
    }
}

/// Whether ANTIC is showing a playfield on its current line.
fn antic_active(a: &Antic) -> bool {
    let mode = a.ir & 0x0f;
    (crate::antic::DISPLAY_START..crate::antic::DISPLAY_END).contains(&a.vline) && mode >= 2 && a.dmactl & 3 != 0
}

/// Playfield at color clock `cc`: the two hi-res halves.
fn playfield(a: &Antic, prior: u8, cc: u16) -> (Pf, Pf) {
    let width = a.dmactl & 3;
    let (lo, hi) = match width {
        1 => (0x40, 0xc0),
        2 => (0x30, 0xd0),
        _ => (0x20, 0xe0),
    };
    if cc < lo || cc >= hi {
        return (Pf::Bak, Pf::Bak);
    }
    let mode = a.ir & 0x0f;
    let hs = a.ir & 0x10 != 0 && mode >= 2;
    let start = if hs {
        let window = (width + 1).min(3);
        [0u16, 0x40, 0x30, 0x20][window as usize] + a.hscrol as u16
    } else {
        lo
    };
    if cc < start {
        return (Pf::Bak, Pf::Bak);
    }
    let off = cc - start;
    let cpb: u16 = match mode {
        2..=5 | 0xd..=0xf => 4,
        6 | 7 | 0xa..=0xc => 8,
        _ => 16,
    };
    let i = (off / cpb) as usize;
    if i >= 48 {
        return (Pf::Bak, Pf::Bak);
    }
    let w = off % cpb;
    let gtia = prior & 0xc0 != 0;
    let same = |p: Pf| (p, p);
    let pair = |b: u8, px: u16| (b >> (6 - 2 * px)) & 3;
    let four = |v: u8, pf3: bool| match v {
        0 => Pf::Bak,
        3 if pf3 => Pf::Color(3),
        v => Pf::Color(v - 1),
    };
    match mode {
        2 | 3 => {
            let name = a.line_buf[i];
            let mut d = a.char_buf[i];
            if mode == 3 {
                let row = a.row();
                if (name & 0x7f < 0x60 && row >= 8) || (name & 0x7f >= 0x60 && row < 2) {
                    d = 0;
                }
            }
            if name & 0x80 != 0 {
                if a.chactl & 1 != 0 {
                    d = 0;
                }
                if a.chactl & 2 != 0 {
                    d = !d;
                }
            }
            if gtia {
                return same(Pf::Gtia(if w < 2 { d >> 4 } else { d & 0x0f }));
            }
            (Pf::Hires(d >> (7 - 2 * w) & 1 != 0), Pf::Hires(d >> (6 - 2 * w) & 1 != 0))
        }
        4 | 5 => {
            let name = a.line_buf[i];
            same(four(pair(a.char_buf[i], w), name & 0x80 != 0))
        }
        6 | 7 => {
            let name = a.line_buf[i];
            let on = a.char_buf[i] >> (7 - w) & 1 != 0;
            same(if on { Pf::Color(name >> 6) } else { Pf::Bak })
        }
        8 => same(four(pair(a.line_buf[i], w / 4), false)),
        9 => same(if a.line_buf[i] >> (7 - w / 2) & 1 != 0 { Pf::Color(0) } else { Pf::Bak }),
        0xa => same(four(pair(a.line_buf[i], w / 2), false)),
        0xb | 0xc => same(if a.line_buf[i] >> (7 - w) & 1 != 0 { Pf::Color(0) } else { Pf::Bak }),
        0xd | 0xe => same(four(pair(a.line_buf[i], w), false)),
        _ => {
            let d = a.line_buf[i];
            if gtia {
                return same(Pf::Gtia(if w < 2 { d >> 4 } else { d & 0x0f }));
            }
            (Pf::Hires(d >> (7 - 2 * w) & 1 != 0), Pf::Hires(d >> (6 - 2 * w) & 1 != 0))
        }
    }
}

/// NTSC palette: colour byte to RGB, from Altirra appendix D's model. Hue 0
/// is grey; hues 1-15 step 25.7 degrees from hue 1 at -57 degrees in I-Q
/// space, saturation 24% of the luma range. Luminance is bits 1-3 (bit 0 is
/// ignored outside the GTIA modes, as on hardware).
pub fn palette() -> [[u8; 3]; 256] {
    let mut p = [[0u8; 3]; 256];
    for (c, out) in p.iter_mut().enumerate() {
        let hue = (c >> 4) as f64;
        let lum = (c & 0x0f) as f64;
        let y = lum / 15.0;
        let (i, q) = if hue == 0.0 {
            (0.0, 0.0)
        } else {
            let ang = (-57.0 + 25.7 * (hue - 1.0)).to_radians();
            (0.24 * ang.cos(), 0.24 * ang.sin())
        };
        let r = y + 0.956 * i + 0.621 * q;
        let g = y - 0.272 * i - 0.647 * q;
        let b = y - 1.107 * i + 1.704 * q;
        let to8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        *out = [to8(r), to8(g), to8(b)];
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_player_is_eight_color_clocks_from_its_hpos() {
        let mut g = Gtia::new();
        g.regs[0x00] = 0x60; // HPOSP0
        g.regs[0x0d] = 0x81; // GRAFP0: leftmost and rightmost bits
        let hits: Vec<u16> = (0x50..0x80).filter(|&cc| g.objects(cc).0 & 1 != 0).collect();
        assert_eq!(hits, vec![0x60, 0x67]);
        g.regs[0x08] = 3; // quad width
        let hits: Vec<u16> = (0x50..0x90).filter(|&cc| g.objects(cc).0 & 1 != 0).collect();
        assert_eq!(hits, vec![0x60, 0x61, 0x62, 0x63, 0x7c, 0x7d, 0x7e, 0x7f]);
    }

    #[test]
    fn a_player_over_playfield_sets_both_collision_registers() {
        let mut g = Gtia::new();
        g.regs[0x01] = 0x60; // HPOSP1
        g.regs[0x0e] = 0xff; // GRAFP1
        g.compose(0x62, Pf::Color(2), Pf::Color(2));
        assert_eq!(g.collisions[0x05], 0x04, "P1PF sees PF2");
        g.write(0x1e, 0);
        assert_eq!(g.collisions[0x05], 0, "HITCLR clears it");
    }

    #[test]
    fn prior_4_puts_playfield_over_players() {
        let mut g = Gtia::new();
        g.regs[COLPM0] = 0x44;
        g.regs[COLPF0] = 0x88;
        g.regs[PRIOR] = 0x01;
        assert_eq!(g.pixel(Pf::Color(0), 1, false), 0x44);
        g.regs[PRIOR] = 0x04;
        assert_eq!(g.pixel(Pf::Color(0), 1, false), 0x88);
    }

    #[test]
    fn hue_zero_is_grey_and_luminance_rises() {
        let p = palette();
        assert_eq!(p[0x00], [0, 0, 0]);
        assert_eq!(p[0x0f], [255, 255, 255]);
        let [r, g, b] = p[0x08];
        assert!(r == g && g == b && r > 100 && r < 160);
        assert_ne!(p[0x48][0], p[0x48][2], "a hue is not grey");
    }
}
