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
use crate::save::{LoadError, ReadCursor, SaveState, WriteCursor};

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
            let (l, r) = if active {
                playfield(antic, self.regs[PRIOR], cc)
            } else {
                (Pf::Bak, Pf::Bak)
            };
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
        (
            self.pixel(l, objs, fifth && missiles != 0),
            self.pixel(r, objs, fifth && missiles != 0),
        )
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

/// The frame buffer is output, redrawn every frame, and the fire buttons are
/// input, set by the frontend every frame, so neither is saved. What is
/// saved is what the chip holds: registers, collision latches, trigger
/// latches, and how far along the current line it has drawn.
impl SaveState for Gtia {
    fn save(&self, w: &mut WriteCursor) {
        w.bytes(&self.regs);
        w.bytes(&self.collisions);
        for v in self.latched {
            w.bool(v);
        }
        w.u16(self.cursor);
    }

    fn load(&mut self, r: &mut ReadCursor) -> Result<(), LoadError> {
        r.bytes(&mut self.regs)?;
        r.bytes(&mut self.collisions)?;
        for v in self.latched.iter_mut() {
            *v = r.bool()?;
        }
        self.cursor = r.u16()?;
        if self.cursor > 228 {
            return Err(LoadError::BadValue("GTIA cursor"));
        }
        Ok(())
    }
}

/// Whether ANTIC is showing a playfield on its current line.
fn antic_active(a: &Antic) -> bool {
    let mode = a.ir & 0x0f;
    (crate::antic::DISPLAY_START..crate::antic::DISPLAY_END).contains(&a.vline)
        && mode >= 2
        && a.dmactl & 3 != 0
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
            (
                Pf::Hires(d >> (7 - 2 * w) & 1 != 0),
                Pf::Hires(d >> (6 - 2 * w) & 1 != 0),
            )
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
        9 => same(if a.line_buf[i] >> (7 - w / 2) & 1 != 0 {
            Pf::Color(0)
        } else {
            Pf::Bak
        }),
        0xa => same(four(pair(a.line_buf[i], w / 2), false)),
        0xb | 0xc => same(if a.line_buf[i] >> (7 - w) & 1 != 0 {
            Pf::Color(0)
        } else {
            Pf::Bak
        }),
        0xd | 0xe => same(four(pair(a.line_buf[i], w), false)),
        _ => {
            let d = a.line_buf[i];
            if gtia {
                return same(Pf::Gtia(if w < 2 { d >> 4 } else { d & 0x0f }));
            }
            (
                Pf::Hires(d >> (7 - 2 * w) & 1 != 0),
                Pf::Hires(d >> (6 - 2 * w) & 1 != 0),
            )
        }
    }
}

/// NTSC palette: colour byte to RGB.
///
/// This is the table the a5200 libretro core uses (libretro/a5200,
/// libretro/libretro.c, a5200_palette_ntsc), taken as is so that a game looks
/// the same here as in the core people compare us against. It is a fixed
/// table rather than a model, and it follows the hue names in
/// the GTIA documentation: hue 1 gold, A turquoise, C green, D yellow-green,
/// F light orange.
///
/// It replaces a generated YIQ model (hue 1 at -57 degrees, 25.7 degree
/// steps), which was close to atari800's own model and came out dull and
/// shifted next to a5200: Pole Position's sky () was pale blue rather
/// than turquoise, its text () blue rather than green, and its grass
/// () green rather than yellow-green.
const NTSC: [u32; 256] = [
    0x000000, 0x252525, 0x343434, 0x4f4f4f, 0x5b5b5b, 0x696969, 0x7b7b7b, 0x8a8a8a, 0xa7a7a7,
    0xb9b9b9, 0xc5c5c5, 0xd0d0d0, 0xd7d7d7, 0xe1e1e1, 0xf4f4f4, 0xffffff, 0x4c3200, 0x623a00,
    0x7b4a00, 0x9a6000, 0xb57400, 0xcc8500, 0xe79e08, 0xf7af10, 0xffc318, 0xffd020, 0xffd828,
    0xffdf30, 0xffe63b, 0xfff440, 0xfffa4b, 0xffff50, 0x992500, 0xaa2500, 0xb42500, 0xd33000,
    0xdd4802, 0xe25009, 0xf46700, 0xf47510, 0xff9e10, 0xffac20, 0xffba3a, 0xffbf50, 0xffc66d,
    0xffd580, 0xffe490, 0xffe699, 0x980c0c, 0x990c0c, 0xc21300, 0xd31300, 0xe23500, 0xe34000,
    0xe44020, 0xe55230, 0xfd7854, 0xff8a6a, 0xff987c, 0xffa48b, 0xffb39e, 0xffc2b2, 0xffd0ba,
    0xffd7c0, 0x990000, 0xa90000, 0xc20400, 0xd30400, 0xda0400, 0xdb0800, 0xe42020, 0xf64040,
    0xfb7070, 0xfb7e7e, 0xfb8f8f, 0xff9f9f, 0xffabab, 0xffb9b9, 0xffc9c9, 0xffcfcf, 0x7e0050,
    0x800050, 0x80005f, 0x950b74, 0xaa2288, 0xbb2f9a, 0xce3fad, 0xd75ab6, 0xe467c3, 0xef72ce,
    0xfb7eda, 0xff8de1, 0xff9de5, 0xffa5e7, 0xffafea, 0xffb8ec, 0x48006c, 0x5c0488, 0x650d90,
    0x7b23a7, 0x933bbf, 0x9d45c9, 0xa74fd3, 0xb25ade, 0xbd65e9, 0xc56df1, 0xce76fa, 0xd583ff,
    0xda90ff, 0xde9cff, 0xe2a9ff, 0xe6b6ff, 0x1b0070, 0x221b8d, 0x3730a2, 0x4841b3, 0x5952c4,
    0x635cce, 0x6f68da, 0x7d76e8, 0x8780f8, 0x938cff, 0x9d97ff, 0xa8a3ff, 0xb3afff, 0xbcb8ff,
    0xc4c1ff, 0xdad1ff, 0x000d7f, 0x0012a7, 0x0018c0, 0x0a2bd1, 0x1b4ae3, 0x2f58f0, 0x3768ff,
    0x4979ff, 0x5b85ff, 0x6d96ff, 0x7fa3ff, 0x8cadff, 0x96b4ff, 0xa8c0ff, 0xb7cbff, 0xc6d6ff,
    0x00295a, 0x003876, 0x004892, 0x005cac, 0x0071c6, 0x0086d0, 0x0a9bdf, 0x1aa8ec, 0x2bb6ff,
    0x3fc2ff, 0x45cbff, 0x59d3ff, 0x7fdaff, 0x8fdeff, 0xa0e2ff, 0xb0ebff, 0x004a00, 0x004c00,
    0x006a20, 0x508e79, 0x409999, 0x009caa, 0x00a1bb, 0x01a4cc, 0x03a5d7, 0x05dae2, 0x18e5ff,
    0x34eaff, 0x49efff, 0x66f2ff, 0x84f4ff, 0x9ef9ff, 0x004a00, 0x005d00, 0x007000, 0x008300,
    0x009500, 0x00ab00, 0x07bd07, 0x0ad00a, 0x1ad540, 0x5af177, 0x82efa7, 0x84edd1, 0x89ffed,
    0x7dffff, 0x93ffff, 0x9bffff, 0x224a03, 0x275304, 0x306405, 0x3c770c, 0x458c11, 0x5aa513,
    0x1bd209, 0x1fdd00, 0x3dcd2d, 0x3dcd30, 0x58cc40, 0x60d350, 0xa2ec55, 0xb3f24a, 0xbbf65d,
    0xc4f870, 0x2e3f0c, 0x364a0f, 0x405615, 0x465f17, 0x57771a, 0x65851c, 0x74931d, 0x8fa525,
    0xadb72c, 0xbcc730, 0xc9d533, 0xd4e03b, 0xe0ec42, 0xeaf645, 0xf0fd47, 0xf4ff6f, 0x552400,
    0x5a2c00, 0x6c3b00, 0x794b00, 0xb97500, 0xbb8500, 0xc1a120, 0xd0b02f, 0xdebe3f, 0xe6c645,
    0xedcd57, 0xf5db62, 0xfbe569, 0xfcee6f, 0xfdf377, 0xfdf37f, 0x5c2700, 0x5c2f00, 0x713b00,
    0x7b4800, 0xb96820, 0xbb7220, 0xc58629, 0xd79633, 0xe6a440, 0xf4b14b, 0xfdc158, 0xffcc55,
    0xffd461, 0xffdd69, 0xffe679, 0xffea98,
];

pub fn palette() -> [[u8; 3]; 256] {
    let mut p = [[0u8; 3]; 256];
    for (out, &v) in p.iter_mut().zip(NTSC.iter()) {
        *out = [(v >> 16) as u8, (v >> 8) as u8, v as u8];
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
        let hits: Vec<u16> = (0x50..0x80)
            .filter(|&cc| g.objects(cc).0 & 1 != 0)
            .collect();
        assert_eq!(hits, vec![0x60, 0x67]);
        g.regs[0x08] = 3; // quad width
        let hits: Vec<u16> = (0x50..0x90)
            .filter(|&cc| g.objects(cc).0 & 1 != 0)
            .collect();
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
        for c in 0..16 {
            let [r, g, b] = p[c];
            assert!(r == g && g == b, "hue 0 is grey at {c:02x}");
        }
        for c in 1..16 {
            assert!(p[c][0] > p[c - 1][0], "grey rises at {c:02x}");
        }
    }

    #[test]
    fn hues_follow_the_gtia_names_on_pole_positions_colours() {
        // The three colours that looked wrong next to a5200 in Pole Position.
        // The old YIQ model fails every one of these.
        let p = palette();
        assert_eq!(p[0xaa], [0x18, 0xe5, 0xff], "sky: turquoise");
        assert_eq!(p[0xa2], [0x00, 0x6a, 0x20], "text: dark green");
        assert_eq!(p[0xd8], [0xad, 0xb7, 0x2c], "grass: yellow-green");
        let [r, g, b] = p[0x18];
        assert!(r > g && g > b && b < 64, "hue 1 is gold");
    }
}
