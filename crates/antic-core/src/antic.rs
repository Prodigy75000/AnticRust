// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! ANTIC: the display-list coprocessor, and the reason timing is not a layer
//! added afterwards.
//!
//! ANTIC owns the bus whenever it needs it. Each scan line is 114 machine
//! cycles, and each cycle either belongs to ANTIC (a DMA slot) or to the CPU.
//! This module plans the slots for a line, performs ANTIC's fetches when the
//! bus offers them, runs the display list, and raises NMIs. The bus asks
//! [`Antic::dma`] at every cycle; a `Some` means the CPU is halted for it.
//!
//! Written from the Altirra Hardware Reference Manual, chapter 4 (display
//! list, NMI timing, WSYNC, VCOUNT, playfield, player/missile and refresh
//! DMA). Mid-line DMACTL changes are not modelled: the plan for a line is
//! fixed when the line starts and when its display-list instruction arrives.

/// Machine cycles per scan line.
pub const CYCLES_PER_LINE: u16 = 114;
/// NTSC scan lines per frame.
pub const LINES_PER_FRAME: u16 = 262;
/// First and one-past-last scan lines of the display list region.
pub const DISPLAY_START: u16 = 8;
pub const DISPLAY_END: u16 = 248;

/// What ANTIC does with one machine cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Free,
    Missile,
    Player(u8),
    DlInstr,
    DlAddrLo,
    DlAddrHi,
    Refresh,
    /// Bitmap byte or character name, into line buffer entry `n`.
    PfLoad(u8),
    /// Character set byte for line buffer entry `n`.
    PfChar(u8),
}

impl Slot {
    /// Whether this slot occupies the bus, halting the CPU.
    pub fn uses_bus(self) -> bool {
        self != Slot::Free
    }
}

/// A fetch ANTIC wants to make this cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaFetch {
    pub addr: u16,
    pub slot: Slot,
}

#[derive(Clone, Debug)]
pub struct Antic {
    // ---- registers ----
    pub dmactl: u8,
    pub chactl: u8,
    pub dlist: u16,
    pub hscrol: u8,
    pub vscrol: u8,
    pub pmbase: u8,
    pub chbase: u8,
    pub nmien: u8,
    pub nmist: u8,

    // ---- beam ----
    pub hpos: u16,
    pub vline: u16,
    pub frame: u64,

    // ---- display list state ----
    /// Instruction register: the current mode line's instruction byte.
    pub ir: u8,
    row: u8,
    end_row: u8,
    /// The next line starts a new mode line.
    need_new: bool,
    /// This line is the first of its mode line (bitmap and name fetches).
    first_line: bool,
    /// Previous mode line had vertical scrolling enabled.
    prev_vs: bool,
    /// Jump-and-wait-for-vertical-blank seen: no more fetches this frame.
    jvb: bool,
    pub memscan: u16,
    dl_addr_lo: u8,

    // ---- line plan and buffers ----
    slots: [Slot; CYCLES_PER_LINE as usize],
    /// Line buffer: bitmap bytes or character names, refreshed on first lines.
    pub line_buf: [u8; 48],
    /// Character set bytes fetched on this line.
    pub char_buf: [u8; 48],
    /// Player and missile bytes fetched for this line, for GTIA.
    pub grafp: [u8; 4],
    pub grafm: u8,

    /// NMI output level.
    pub nmi_line: bool,
}

impl Default for Antic {
    fn default() -> Self {
        Antic {
            dmactl: 0,
            chactl: 0,
            dlist: 0,
            hscrol: 0,
            vscrol: 0,
            pmbase: 0,
            chbase: 0,
            nmien: 0,
            nmist: 0x1f,
            hpos: 0,
            vline: 0,
            frame: 0,
            ir: 0,
            row: 0,
            end_row: 0,
            need_new: true,
            first_line: false,
            prev_vs: false,
            jvb: false,
            memscan: 0,
            dl_addr_lo: 0,
            slots: [Slot::Free; CYCLES_PER_LINE as usize],
            line_buf: [0; 48],
            char_buf: [0; 48],
            grafp: [0; 4],
            grafm: 0,
            nmi_line: false,
        }
    }
}

/// Scan lines per mode line, by IR mode 2-F.
fn mode_height(mode: u8) -> u8 {
    match mode {
        2 | 4 | 6 | 8 => 8,
        3 => 10,
        5 | 7 => 16,
        9 | 0xa => 4,
        0xb | 0xd => 2,
        _ => 1, // C, E, F
    }
}

impl Antic {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset clears NMIEN, DMACTL and the playfield DMA clock; the beam keeps
    /// running.
    pub fn reset(&mut self) {
        self.nmien = 0;
        self.dmactl = 0;
    }

    fn in_display(&self) -> bool {
        (DISPLAY_START..DISPLAY_END).contains(&self.vline)
    }

    // ---- CPU-visible registers ----

    /// VCOUNT: bits 1-8 of the line counter, updated at cycle 111 of the
    /// previous line, with the one-cycle $83 at the end of the frame.
    pub fn vcount(&self) -> u8 {
        if self.hpos >= 111 {
            if self.vline == LINES_PER_FRAME - 1 {
                if self.hpos == 111 {
                    (LINES_PER_FRAME >> 1) as u8
                } else {
                    0
                }
            } else {
                ((self.vline + 1) >> 1) as u8
            }
        } else {
            (self.vline >> 1) as u8
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr & 0x0f {
            0x0b => self.vcount(),
            0x0c => 0, // PENH: no light pen on the 5200
            0x0d => 0xff,
            0x0f => self.nmist | 0x1f,
            _ => 0xff,
        }
    }

    /// Register write. WSYNC is the bus's business, so it is not handled here.
    pub fn write(&mut self, addr: u16, v: u8) {
        match addr & 0x0f {
            0x00 => self.dmactl = v & 0x3f,
            0x01 => self.chactl = v & 0x07,
            0x02 => self.dlist = (self.dlist & 0xff00) | v as u16,
            0x03 => self.dlist = (self.dlist & 0x00ff) | ((v as u16) << 8),
            0x04 => self.hscrol = v & 0x0f,
            0x05 => self.vscrol = v & 0x0f,
            0x07 => self.pmbase = v,
            0x09 => self.chbase = v,
            0x0e => self.nmien = v & 0xc0,
            0x0f => self.nmist = 0x1f, // NMIRES
            _ => {}
        }
    }

    // ---- per-cycle operation ----

    /// The fetch ANTIC makes on the current cycle, if any. Cycle 1's slot is
    /// the display-list instruction; the rest of the line is planned once it
    /// arrives (see [`Antic::complete`]).
    pub fn dma(&self) -> Option<DmaFetch> {
        let slot = self.slots[self.hpos as usize];
        let addr = match slot {
            Slot::Free => return None,
            Slot::Refresh => return Some(DmaFetch { addr: 0, slot }),
            Slot::Missile => self.pm_addr(None),
            Slot::Player(n) => self.pm_addr(Some(n)),
            Slot::DlInstr | Slot::DlAddrLo | Slot::DlAddrHi => self.dlist,
            Slot::PfLoad(_) => self.memscan,
            Slot::PfChar(i) => self.char_addr(self.line_buf[i as usize]),
        };
        Some(DmaFetch { addr, slot })
    }

    /// Deliver the byte for this cycle's fetch.
    pub fn complete(&mut self, f: DmaFetch, v: u8) {
        match f.slot {
            Slot::Free | Slot::Refresh => {}
            Slot::Missile => self.grafm = v,
            Slot::Player(n) => self.grafp[n as usize] = v,
            Slot::DlInstr => {
                self.ir = v;
                self.bump_dlist();
                self.start_mode_line(true);
            }
            Slot::DlAddrLo => {
                self.dl_addr_lo = v;
                self.bump_dlist();
            }
            Slot::DlAddrHi => {
                self.bump_dlist();
                let addr = u16::from_le_bytes([self.dl_addr_lo, v]);
                if self.ir & 0x0f == 1 {
                    self.dlist = addr;
                    if self.ir & 0x40 != 0 {
                        self.jvb = true;
                    }
                } else {
                    self.memscan = addr;
                }
            }
            Slot::PfLoad(i) => {
                self.line_buf[i as usize] = v;
                self.memscan = (self.memscan & 0xf000) | (self.memscan.wrapping_add(1) & 0x0fff);
            }
            Slot::PfChar(i) => self.char_buf[i as usize] = v,
        }
    }

    /// The display list pointer increments in its low 10 bits only.
    fn bump_dlist(&mut self) {
        self.dlist = (self.dlist & 0xfc00) | (self.dlist.wrapping_add(1) & 0x03ff);
    }

    fn pm_addr(&self, player: Option<u8>) -> u16 {
        let single = self.dmactl & 0x10 != 0;
        let v = self.vline;
        match (single, player) {
            (true, None) => (((self.pmbase & 0xf8) as u16) << 8) + 0x300 + v,
            (true, Some(n)) => (((self.pmbase & 0xf8) as u16) << 8) + 0x400 + 0x100 * n as u16 + v,
            (false, None) => (((self.pmbase & 0xfc) as u16) << 8) + 0x180 + (v >> 1),
            (false, Some(n)) => {
                (((self.pmbase & 0xfc) as u16) << 8) + 0x200 + 0x80 * n as u16 + (v >> 1)
            }
        }
    }

    fn char_addr(&self, name: u8) -> u16 {
        let mode = self.ir & 0x0f;
        let mut crow = match mode {
            5 | 7 => self.row >> 1,
            _ => self.row,
        } & 7;
        if self.chactl & 0x04 != 0 {
            crow = 7 - crow;
        }
        if mode >= 6 {
            (((self.chbase & 0xfe) as u16) << 8) | (((name & 0x3f) as u16) << 3) | crow as u16
        } else {
            (((self.chbase & 0xfc) as u16) << 8) | (((name & 0x7f) as u16) << 3) | crow as u16
        }
    }

    /// Advance one machine cycle, after this cycle's bus access. Line and
    /// frame events happen here.
    pub fn advance(&mut self) {
        match self.hpos {
            7 => self.nmi_status(),
            8 => {
                // NMIST set on cycle 7; the line is pulled from cycle 8.
                let dli = self.nmist & 0x80 != 0 && self.dli_this_line();
                let vbi = self.vline == DISPLAY_END;
                if (dli && self.nmien & 0x80 != 0) || (vbi && self.nmien & 0x40 != 0) {
                    self.nmi_line = true;
                }
            }
            _ => {}
        }
        self.hpos += 1;
        if self.hpos == CYCLES_PER_LINE {
            self.hpos = 0;
            self.end_line();
            self.begin_line();
        }
    }

    fn dli_this_line(&self) -> bool {
        self.in_display() && self.ir & 0x80 != 0 && self.row == self.end_row
    }

    fn nmi_status(&mut self) {
        if self.vline == DISPLAY_END {
            self.nmist = (self.nmist & !0x80) | 0x40;
        } else if self.dli_this_line() {
            self.nmist = (self.nmist & !0x40) | 0x80;
        }
    }

    fn end_line(&mut self) {
        self.nmi_line = false;
        if self.in_display() {
            if self.row == self.end_row {
                self.need_new = true;
            } else {
                self.row = (self.row + 1) & 0x0f;
            }
            self.first_line = false;
        }
        self.vline += 1;
        if self.vline == LINES_PER_FRAME {
            self.vline = 0;
            self.frame += 1;
        }
    }

    /// Plan the line's fixed slots: missiles, display list, players, and,
    /// when no instruction is fetched, the rest of the line too.
    pub fn begin_line(&mut self) {
        self.slots = [Slot::Free; CYCLES_PER_LINE as usize];
        if self.vline == DISPLAY_START {
            self.jvb = false;
            self.need_new = true;
            self.prev_vs = false;
        }
        let display = self.in_display();
        if display {
            let players = self.dmactl & 0x08 != 0;
            if players || self.dmactl & 0x04 != 0 {
                self.slots[0] = Slot::Missile;
            }
            if players {
                for n in 0..4 {
                    self.slots[2 + n as usize] = Slot::Player(n);
                }
            }
            if self.need_new {
                if self.dmactl & 0x20 != 0 && !self.jvb {
                    // The instruction arrives at cycle 1; complete() plans the rest.
                    self.slots[1] = Slot::DlInstr;
                    return;
                }
                // No fetch: the previous instruction replays, without its
                // address fetch.
                self.start_mode_line(false);
                return;
            }
        }
        self.plan_rest();
    }

    /// A new mode line begins with the instruction in `ir`.
    fn start_mode_line(&mut self, fetched: bool) {
        self.need_new = false;
        self.first_line = true;
        let mode = self.ir & 0x0f;
        let (height, vs) = match mode {
            0 => (((self.ir >> 4) & 7) + 1, false),
            1 => (1, false),
            m => (mode_height(m), self.ir & 0x20 != 0),
        };
        let start = if vs && !self.prev_vs { self.vscrol } else { 0 };
        let end = if !vs && self.prev_vs && mode >= 2 { self.vscrol } else { height - 1 };
        self.row = start & 0x0f;
        self.end_row = end & 0x0f;
        self.prev_vs = vs;
        if fetched && (mode == 1 || (mode >= 2 && self.ir & 0x40 != 0)) {
            self.slots[6] = Slot::DlAddrLo;
            self.slots[7] = Slot::DlAddrHi;
        }
        self.plan_rest();
    }

    /// Playfield and refresh slots for the rest of the line.
    fn plan_rest(&mut self) {
        if self.in_display() && !self.jvb_blank() {
            self.plan_playfield();
        }
        // Refresh: nine requests at 25, 29, ... 57. A blocked one waits for
        // the next free cycle; only one can wait, extra ones are dropped.
        let mut waiting = false;
        for c in 25..CYCLES_PER_LINE as usize {
            if c <= 57 && (c - 25) % 4 == 0 {
                waiting = true;
            }
            if waiting && self.slots[c] == Slot::Free {
                self.slots[c] = Slot::Refresh;
                waiting = false;
            }
        }
    }

    /// A replayed jump (including after JVB) shows a blank line.
    fn jvb_blank(&self) -> bool {
        self.ir & 0x0f <= 1
    }

    fn plan_playfield(&mut self) {
        let mode = self.ir & 0x0f;
        let width = self.dmactl & 3;
        if mode < 2 || width == 0 {
            return;
        }
        let hs = self.ir & 0x10 != 0;
        let window = if hs { (width + 1).min(3) } else { width };
        let delay = if hs { (self.hscrol >> 1) as usize } else { 0 };
        let span = [0usize, 64, 80, 96][window as usize];
        let char_mode = mode <= 7;
        let rate = match mode {
            2..=5 | 0xd..=0xf => 2,
            6 | 7 | 0xa..=0xc => 4,
            _ => 8,
        };
        let count = span / rate;
        if char_mode {
            let name_start = [0usize, 26, 18, 10][window as usize] + delay;
            for i in 0..count {
                let n = name_start + rate * i;
                if self.first_line && n < 106 {
                    self.slots[n] = Slot::PfLoad(i as u8);
                }
                let d = n + 3;
                if d < 106 {
                    self.slots[d] = Slot::PfChar(i as u8);
                }
            }
        } else if self.first_line {
            let start = [0usize, 28, 20, 12][window as usize] + delay;
            for i in 0..count {
                let c = start + rate * i;
                if c < 106 {
                    self.slots[c] = Slot::PfLoad(i as u8);
                }
            }
        }
    }

    /// The planned slot for cycle `c` of the current line.
    pub fn slot(&self, c: u16) -> Slot {
        self.slots[c as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run whole lines with a memory that is all `fill` except a display list.
    fn run_lines(a: &mut Antic, mem: &[u8; 0x10000], lines: u32) -> Vec<u32> {
        let mut busy_per_line = Vec::new();
        for _ in 0..lines {
            let mut busy = 0;
            for _ in 0..CYCLES_PER_LINE {
                if let Some(f) = a.dma() {
                    busy += 1;
                    a.complete(f, mem[f.addr as usize]);
                }
                a.advance();
            }
            busy_per_line.push(busy);
        }
        busy_per_line
    }

    #[test]
    fn a_blank_frame_still_spends_nine_refresh_cycles_a_line() {
        let mut a = Antic::new();
        a.begin_line();
        let mem = Box::new([0u8; 0x10000]);
        let busy = run_lines(&mut a, &mem, 262);
        assert!(busy.iter().all(|&b| b == 9), "{busy:?}");
    }

    #[test]
    fn vcount_ticks_every_two_lines_and_wraps_at_131() {
        let mut a = Antic::new();
        a.begin_line();
        let mem = Box::new([0u8; 0x10000]);
        a.vline = 261;
        a.hpos = 110;
        assert_eq!(a.vcount(), 130);
        a.hpos = 111;
        assert_eq!(a.vcount(), 0x83);
        a.hpos = 112;
        assert_eq!(a.vcount(), 0);
        let _ = mem;
    }

    #[test]
    fn mode_e_normal_width_fetches_forty_bytes_on_its_line() {
        // DL at $1000: LMS mode E from $2000, then JVB.
        let mut mem = Box::new([0u8; 0x10000]);
        mem[0x1000..0x1006].copy_from_slice(&[0x4e, 0x00, 0x20, 0x41, 0x00, 0x10]);
        let mut a = Antic::new();
        a.dmactl = 0x22; // DL DMA + normal playfield
        a.dlist = 0x1000;
        a.vline = DISPLAY_START;
        a.begin_line();
        let busy = run_lines(&mut a, &mem, 2);
        // instr + 2 address bytes + 40 playfield + 9 refresh = 52.
        assert_eq!(busy[0], 52);
        assert_eq!(a.memscan, 0x2028);
        // Second line: the JVB, one instruction byte and two address bytes.
        assert_eq!(busy[1], 3 + 9);
        assert_eq!(a.dlist, 0x1000);
    }

    #[test]
    fn a_dli_raises_nmi_on_the_last_line_of_its_mode_line() {
        // $F0: blank 8 lines with DLI. NMIST bit 7 should appear on line 15.
        let mut mem = Box::new([0u8; 0x10000]);
        mem[0x1000] = 0xf0;
        mem[0x1001] = 0x41;
        let mut a = Antic::new();
        a.dmactl = 0x20;
        a.nmien = 0x80;
        a.dlist = 0x1000;
        a.vline = DISPLAY_START;
        a.begin_line();
        let mut nmi_lines = Vec::new();
        for _ in 0..10 {
            let line = a.vline;
            for _ in 0..CYCLES_PER_LINE {
                if let Some(f) = a.dma() {
                    a.complete(f, mem[f.addr as usize]);
                }
                if a.nmi_line && !nmi_lines.contains(&line) {
                    nmi_lines.push(line);
                }
                a.advance();
            }
        }
        assert_eq!(nmi_lines, vec![DISPLAY_START + 7]);
    }

    #[test]
    fn vbi_sets_nmist_bit_6_at_line_248() {
        let mut a = Antic::new();
        a.nmien = 0x40;
        a.vline = DISPLAY_END;
        a.begin_line();
        for _ in 0..9 {
            a.advance();
        }
        assert_eq!(a.nmist & 0xc0, 0x40);
        assert!(a.nmi_line);
    }
}
