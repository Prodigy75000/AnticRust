// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! The HLE BIOS: a synthesised 2K image that runs instead of the real one.
//!
//! The census (docs/notes/BIOS-CENSUS.md) showed the intercept has to be BOTH
//! code at fixed addresses and data, so this is one image rather than traps
//! in the CPU loop: our own 6502 routines, assembled here, at the entry
//! points cartridges are measured to use, plus the font and the vector table.
//!
//! | address | what | why it must be there |
//! |---|---|---|
//! | $F800 | font, 128 glyphs | 45 titles display it, 24 copy from it |
//! | $FC00 | IRQ entry, `JMP ($0200)` | hardware IRQ vector |
//! | $FC03 | IRQ dispatcher | default $0200; 54 titles take IRQs |
//! | $FCA2 | NMI dispatcher | hardware NMI vector |
//! | $FCB2 | restore Y, X, A and RTI | 15 titles jump here |
//! | $FCB7 | RTI | |
//! | $FCB8 | VBI: frame counter, attract, shadows, pots | default $0202 |
//! | $FCBD | the VBI after its register saves | Blueprint enters here |
//! | $FD02 | keypad decode, then `JMP ($020A)` at $FD10 | 34 titles |
//! | $FD13 | keypad code to key table | read by the decode |
//! | $FD23 | reset | hardware vector; Zaxxon jumps here |
//! | $FE95 | default RAM vectors, 12 bytes | 4 titles copy them |
//! | $FEA1 | default DLI | default $0206 |
//! | $FFFA | NMI, RESET, IRQ vectors | |
//!
//! Boot: a cart with $BFFD = $FF is entered at once, as on hardware. Any
//! other cart gets the post-boot state the real BIOS leaves, synthesised
//! directly from the values `entrystate` measured (identical across all 29
//! full-boot titles), without the 4.25 s logo. Whether the logo is shown is
//! the owner's decision and still open.

use std::collections::HashMap;

use crate::disasm::{decode, Mode};

/// Operand for the assembler: a number, or a label resolved at the end.
#[derive(Clone, Copy)]
enum Arg {
    None,
    Val(u16),
    Label(&'static str),
}

struct Asm {
    img: [u8; 0x800],
    pc: u16,
    labels: HashMap<&'static str, u16>,
    /// (where the operand goes, label, mode) to patch at the end.
    fixups: Vec<(u16, &'static str, Mode)>,
}

/// Official opcode for a mnemonic and mode. Undocumented duplicates are never
/// chosen, so the image only contains instructions any 6502 runs alike.
fn opcode(name: &str, mode: Mode) -> u8 {
    const ILLEGAL: &[&str] = &["SLO", "RLA", "SRE", "RRA", "SAX", "LAX", "DCP", "ISC", "ANC", "ALR", "ARR", "XAA", "LXA", "SBX", "TAS", "LAS", "SHY", "SHX", "SHA", "JAM"];
    (0..=255u8)
        .find(|&op| {
            let (n, m) = decode(op);
            n == name && m == mode && !ILLEGAL.contains(&n) && op != 0xeb && (n != "NOP" || op == 0xea)
        })
        .unwrap_or_else(|| panic!("no opcode for {name} {mode:?}"))
}

impl Asm {
    fn new() -> Self {
        Asm { img: [0xff; 0x800], pc: 0xf800, labels: HashMap::new(), fixups: Vec::new() }
    }

    /// Move to a fixed address. Code must never run into the next fixed
    /// block, so moving backwards is an error.
    fn org(&mut self, addr: u16) {
        assert!(addr >= self.pc, "code at ${:04X} overran ${addr:04X}", self.pc);
        self.pc = addr;
    }

    fn here(&mut self, name: &'static str) {
        assert!(self.labels.insert(name, self.pc).is_none(), "label {name} twice");
    }

    fn byte(&mut self, b: u8) {
        assert!(self.pc >= 0xf800, "assembled past $FFFF");
        self.img[(self.pc - 0xf800) as usize] = b;
        // Wraps to $0000 after the last vector byte; nothing may follow it.
        self.pc = self.pc.wrapping_add(1);
    }

    fn bytes(&mut self, bs: &[u8]) {
        for &b in bs {
            self.byte(b);
        }
    }

    fn i(&mut self, name: &str, mode: Mode, arg: Arg) {
        self.byte(opcode(name, mode));
        let at = self.pc;
        let v = match arg {
            Arg::None => 0,
            Arg::Val(v) => v,
            Arg::Label(l) => {
                self.fixups.push((at, l, mode));
                0
            }
        };
        match mode.len() {
            1 => {}
            2 => self.byte(v as u8),
            _ => self.bytes(&v.to_le_bytes()),
        }
    }

    fn finish(mut self) -> [u8; 0x800] {
        for (at, l, mode) in std::mem::take(&mut self.fixups) {
            let target = *self.labels.get(l).unwrap_or_else(|| panic!("undefined label {l}"));
            let o = (at - 0xf800) as usize;
            if mode == Mode::Rel {
                let d = target as i32 - (at as i32 + 1);
                assert!((-128..=127).contains(&d), "branch to {l} out of range");
                self.img[o] = d as i8 as u8;
            } else {
                self.img[o..o + 2].copy_from_slice(&target.to_le_bytes());
            }
        }
        self.img
    }
}

use Arg::{Label as L, None as N, Val as V};
use Mode::*;

// Hardware and RAM addresses the routines use.
const IRQST: u16 = 0xe80e; // read: pending, active low; write: IRQEN
const KBCODE: u16 = 0xe809;
const POTGO: u16 = 0xe80b;
const POT0: u16 = 0xe800;
const SKCTL: u16 = 0xe80f;
const NMIST: u16 = 0xd40f; // read: status; write: NMIRES
const NMIEN: u16 = 0xd40e;
const DMACTL: u16 = 0xd400;
const DLISTL: u16 = 0xd402;
const DLISTH: u16 = 0xd403;
const CHBASE: u16 = 0xd409;
const COLPM0: u16 = 0xc012;
const COLPF1: u16 = 0xc017;
const COLPF0: u16 = 0xc016;
const WSYNC: u16 = 0xd40a;
const POKMSK: u16 = 0x00; // IRQEN shadow
const RTCLOK_HI: u16 = 0x01;
const RTCLOK: u16 = 0x02;
const CRITIC: u16 = 0x03;
const ATRACT: u16 = 0x04;
const SDLSTL: u16 = 0x05;
const SDLSTH: u16 = 0x06;
const SDMCTL: u16 = 0x07;
const PCOLR0: u16 = 0x08; // nine colour shadows, $08-$10
const PADDL0: u16 = 0x11; // eight pot shadows, $11-$18

/// RAM vectors the dispatchers jump through.
const VIMIRQ: u16 = 0x0200;
const VVBLKI: u16 = 0x0202;
const VVBLKD: u16 = 0x0204;
const VDSLST: u16 = 0x0206;
const VKYBDI: u16 = 0x0208;
const VKYBDF: u16 = 0x020a;
const VTRIGR: u16 = 0x020c;
const VBRKOP: u16 = 0x020e;
const VSERIN: u16 = 0x0210;
const VSEROR: u16 = 0x0212;
const VSEROC: u16 = 0x0214;
const VTIMR1: u16 = 0x0216;
const VTIMR2: u16 = 0x0218;
const VTIMR4: u16 = 0x021a;

/// Keypad scan code (KBCODE bits 1-4) to key value: digits as themselves,
/// * = $0A, # = $0B, Start = $0C, Pause = $0D, Reset = $0E, none = $FF.
/// Follows from the keypad matrix (Altirra 13.3).
pub const KEY_TABLE: [u8; 16] = [
    0xff, 0x0b, 0x00, 0x0a, 0x0e, 0x09, 0x08, 0x07, 0x0d, 0x06, 0x05, 0x04, 0x0c, 0x03, 0x02, 0x01,
];

/// The post-boot state the real BIOS leaves, as `entrystate` measured it.
pub mod post_boot {
    pub const RTCLOK: u8 = 0xff;
    pub const SDLSTH: u8 = 0x20;
    pub const SDMCTL: u8 = 0x22;
    pub const PCOLR0: u8 = 0x1d;
    pub const COLOR1: u8 = 0x0f; // shadow $0D, to COLPF1
    pub const PADDL: u8 = 0xe4;
    pub const CHBASE: u8 = 0xf8;
    pub const SKCTL: u8 = 0x02;
    pub const NMIEN: u8 = 0xc0;
    pub const A: u8 = 0x02;
    pub const X: u8 = 0xff;
    pub const Y: u8 = 0x1b;
    pub const P: u8 = 0x27;
}

/// Acknowledge one POKEY IRQ source and leave through its vector.
fn ack(a: &mut Asm, bit: u8, vector: u16) {
    a.i("LDA", Imm, V(!bit as u16 & 0xff));
    a.i("STA", Abs, V(IRQST));
    a.i("LDA", Zp, V(POKMSK));
    a.i("STA", Abs, V(IRQST));
    a.i("JMP", Ind, V(vector));
}

/// Build the 2K image for $F800-$FFFF.
pub fn image() -> [u8; 0x800] {
    let mut a = Asm::new();

    // ---- $F800: font ----
    a.bytes(&crate::font::FONT);

    // ---- $FC00: IRQ entry ----
    a.org(0xfc00);
    a.i("JMP", Ind, V(VIMIRQ));

    // ---- $FC03: IRQ dispatcher ----
    // Order and layout follow the hardware's dispatcher, because its cycle
    // count per path is observable: a game that reads RANDOM or VCOUNT after
    // an IRQ sees the difference, and `hlediff --swap` did. Order: serial
    // in, break (top button), keypad, serial out, transmit complete (only if
    // enabled in POKMSK, the bit is not latched), timers 1, 2, 4, then BRK.
    // Each source is acknowledged by dropping it from IRQEN and restoring
    // POKMSK, and left through its RAM vector with A on the stack.
    a.here("irq");
    a.i("PHA", Imp, N);
    a.i("LDA", Imm, V(0x20));
    a.i("BIT", Abs, V(IRQST)); // N = bit 7 (break), V = bit 6 (keypad)
    a.i("BNE", Rel, L("irq_bk"));
    ack(&mut a, 0x20, VSERIN);
    a.here("irq_bk");
    assert_eq!(a.pc, 0xfc18);
    a.i("BPL", Rel, L("irq_break"));
    a.i("BVC", Rel, L("irq_key"));
    a.i("LDA", Imm, V(0x10));
    a.i("AND", Abs, V(IRQST));
    a.i("BNE", Rel, L("irq_xmit"));
    ack(&mut a, 0x10, VSEROR);
    a.here("irq_xmit");
    assert_eq!(a.pc, 0xfc30);
    a.i("LDA", Imm, V(0x08));
    a.i("AND", Zp, V(POKMSK));
    a.i("BEQ", Rel, L("irq_t1"));
    a.i("AND", Abs, V(IRQST));
    a.i("BNE", Rel, L("irq_t1"));
    ack(&mut a, 0x08, VSEROC);
    a.here("irq_t1");
    assert_eq!(a.pc, 0xfc48);
    a.i("LDA", Abs, V(IRQST));
    a.i("ROR", Acc, N);
    a.i("BCS", Rel, L("irq_t2"));
    ack(&mut a, 0x01, VTIMR1);
    a.here("irq_t2");
    a.i("ROR", Acc, N);
    a.i("BCS", Rel, L("irq_t4"));
    ack(&mut a, 0x02, VTIMR2);
    a.here("irq_t4");
    a.i("ROR", Acc, N);
    a.i("BCS", Rel, L("irq_brk"));
    ack(&mut a, 0x04, VTIMR4);
    // Nothing pending: a BRK if the pushed P has B set, else spurious.
    a.here("irq_brk");
    assert_eq!(a.pc, 0xfc7b);
    a.i("TXA", Imp, N);
    a.i("PHA", Imp, N);
    a.i("TSX", Imp, N);
    a.i("LDA", Abx, V(0x0103));
    a.i("AND", Imm, V(0x10));
    a.i("BEQ", Rel, L("pla_x_a_rti"));
    a.i("JMP", Ind, V(VBRKOP));
    a.here("irq_break");
    assert_eq!(a.pc, 0xfc88);
    ack(&mut a, 0x80, VTRIGR);
    a.here("irq_key");
    assert_eq!(a.pc, 0xfc95);
    ack(&mut a, 0x40, VKYBDI);

    // ---- $FCA2: NMI dispatcher ----
    a.org(0xfca2);
    a.i("BIT", Abs, V(NMIST));
    a.i("STA", Abs, V(NMIST));
    a.i("BMI", Rel, L("dli"));
    a.i("BVC", Rel, L("rti"));
    a.i("JMP", Ind, V(VVBLKI));
    a.here("dli");
    a.i("JMP", Ind, V(VDSLST));

    // ---- $FCB2: the shared exit, restoring Y, X, A ----
    a.org(0xfcb2);
    a.here("exit_tail");
    a.i("PLA", Imp, N);
    a.i("TAY", Imp, N);
    a.here("pla_x_a_rti");
    a.i("PLA", Imp, N);
    a.i("TAX", Imp, N);
    a.i("PLA", Imp, N);
    a.here("rti");
    assert_eq!(a.pc, 0xfcb7);
    a.i("RTI", Imp, N);

    // ---- $FCB8: VBI ----
    a.i("PHA", Imp, N);
    a.i("TXA", Imp, N);
    a.i("PHA", Imp, N);
    a.i("TYA", Imp, N);
    a.i("PHA", Imp, N);
    assert_eq!(a.pc, 0xfcbd);
    a.i("INC", Zp, V(RTCLOK));
    a.i("BNE", Rel, L("vbi_crit"));
    a.i("INC", Zp, V(ATRACT));
    a.i("INC", Zp, V(RTCLOK_HI));
    a.here("vbi_crit");
    a.i("LDA", Zp, V(CRITIC));
    a.i("BNE", Rel, L("exit_tail"));
    // Shadows to the chips.
    a.i("LDA", Zp, V(SDLSTH));
    a.i("STA", Abs, V(DLISTH));
    a.i("LDA", Zp, V(SDLSTL));
    a.i("STA", Abs, V(DLISTL));
    a.i("LDA", Zp, V(SDMCTL));
    a.i("STA", Abs, V(DMACTL));
    // Attract mode: once the counter passes $80 it stays there and colours
    // are dimmed and shifted by the high clock byte.
    a.i("LDY", Zp, V(ATRACT));
    a.i("BPL", Rel, L("vbi_col"));
    a.i("LDY", Imm, V(0x80));
    a.i("STY", Zp, V(ATRACT));
    a.here("vbi_col");
    a.i("LDX", Imm, V(8));
    a.here("vbi_col_loop");
    a.i("LDA", Zpx, V(PCOLR0));
    a.i("CPY", Imm, V(0x80));
    a.i("BCC", Rel, L("vbi_col_put"));
    a.i("EOR", Zp, V(RTCLOK_HI));
    a.i("AND", Imm, V(0xf6));
    a.here("vbi_col_put");
    a.i("STA", Abx, V(COLPM0));
    a.i("DEX", Imp, N);
    a.i("BPL", Rel, L("vbi_col_loop"));
    // Pots to their shadows, then restart the scan.
    a.i("LDX", Imm, V(7));
    a.here("vbi_pot");
    a.i("LDA", Abx, V(POT0));
    a.i("STA", Zpx, V(PADDL0));
    a.i("DEX", Imp, N);
    a.i("BPL", Rel, L("vbi_pot"));
    a.i("STA", Abs, V(POTGO));
    a.i("JMP", Ind, V(VVBLKD));

    // ---- $FD02: keypad decode ----
    a.org(0xfd02);
    a.i("TXA", Imp, N);
    a.i("PHA", Imp, N);
    a.i("TYA", Imp, N);
    a.i("PHA", Imp, N);
    a.i("LDA", Abs, V(KBCODE));
    a.i("LSR", Acc, N);
    a.i("AND", Imm, V(0x0f));
    a.i("TAX", Imp, N);
    a.i("LDA", Abx, V(0xfd13));
    assert_eq!(a.pc, 0xfd10);
    a.i("JMP", Ind, V(VKYBDF));
    a.bytes(&KEY_TABLE);

    // ---- $FD23: reset ----
    a.org(0xfd23);
    a.i("SEI", Imp, N);
    a.i("CLD", Imp, N);
    a.i("LDX", Imm, V(0xff));
    a.i("TXS", Imp, N);
    a.i("LDA", Abs, V(0xbffd));
    a.i("CMP", Imm, V(0xff));
    a.i("BNE", Rel, L("boot"));
    a.i("JMP", Ind, V(0xbffe));
    a.here("boot");
    // Clear the three chips' register blocks and zero page.
    a.i("INX", Imp, N);
    a.i("TXA", Imp, N);
    a.here("clr");
    a.i("STA", Abx, V(0xe800));
    a.i("STA", Abx, V(0xc000));
    a.i("STA", Abx, V(0xd400));
    a.i("STA", Zpx, V(0x00));
    a.i("INX", Imp, N);
    a.i("BNE", Rel, L("clr"));
    // RAM vectors from the default table.
    a.i("LDX", Imm, V(11));
    a.here("vec");
    a.i("LDA", Abx, V(0xfe95));
    a.i("STA", Abx, V(VIMIRQ));
    a.i("DEX", Imp, N);
    a.i("BPL", Rel, L("vec"));
    // The measured post-boot state, without the logo.
    let set = |a: &mut Asm, v: u8, at: u16, zp: bool| {
        a.i("LDA", Imm, V(v as u16));
        a.i("STA", if zp { Zp } else { Abs }, V(at));
    };
    set(&mut a, post_boot::RTCLOK, RTCLOK, true);
    set(&mut a, post_boot::SDLSTH, SDLSTH, true);
    set(&mut a, post_boot::SDMCTL, SDMCTL, true);
    set(&mut a, post_boot::PCOLR0, PCOLR0, true);
    set(&mut a, post_boot::COLOR1, PCOLR0 + 5, true);
    a.i("LDA", Imm, V(post_boot::PADDL as u16));
    a.i("LDX", Imm, V(7));
    a.here("pad");
    a.i("STA", Zpx, V(PADDL0));
    a.i("DEX", Imp, N);
    a.i("BPL", Rel, L("pad"));
    set(&mut a, post_boot::SDLSTH, DLISTH, false);
    set(&mut a, post_boot::SDMCTL, DMACTL, false);
    set(&mut a, post_boot::PCOLR0, COLPM0, false);
    set(&mut a, post_boot::COLOR1, COLPF1, false);
    set(&mut a, post_boot::CHBASE, CHBASE, false);
    set(&mut a, post_boot::SKCTL, SKCTL, false);
    set(&mut a, post_boot::NMIEN, NMIEN, false);
    // Registers as the real boot hands them over: P via the stack so that
    // loading A, X, Y does not disturb the flags.
    a.i("LDA", Imm, V(post_boot::P as u16));
    a.i("PHA", Imp, N);
    a.i("LDA", Imm, V(post_boot::A as u16));
    a.i("LDX", Imm, V(post_boot::X as u16));
    a.i("LDY", Imm, V(post_boot::Y as u16));
    a.i("PLP", Imp, N);
    a.i("JMP", Ind, V(0xbffe));

    // ---- $FE95: default RAM vectors, copied to $0200 ----
    a.org(0xfe95);
    for v in [0xfc03u16, 0xfcb8, 0xfcb2, 0xfea1, 0xfd02, 0xfcb2] {
        a.bytes(&v.to_le_bytes());
    }

    // ---- $FEA1: default DLI ----
    // The rolling colour bars of the logo screen. Titles that enter while
    // the logo's display list is still up run it for their first frames
    // (hlediff --swap shows it at frame 1). Behaviour: from the colour in
    // $08, put a colour on PF0 for each of $72 lines, stepping down by 2 and
    // wrapping below $10 to $FE; then advance $08, never below $10.
    assert_eq!(a.pc, 0xfea1);
    a.i("PHA", Imp, N);
    a.i("TXA", Imp, N);
    a.i("PHA", Imp, N);
    a.i("TYA", Imp, N);
    a.i("PHA", Imp, N);
    a.i("LDX", Zp, V(PCOLR0));
    a.i("LDY", Imm, V(0x72));
    a.here("dli_line");
    a.i("CPX", Imm, V(0x10));
    a.i("BCS", Rel, L("dli_put"));
    a.i("LDX", Imm, V(0xfe));
    a.here("dli_put");
    a.i("STX", Abs, V(WSYNC));
    a.i("STX", Abs, V(COLPF0));
    a.i("DEX", Imp, N);
    a.i("DEX", Imp, N);
    a.i("DEY", Imp, N);
    a.i("BNE", Rel, L("dli_line"));
    a.i("INC", Zp, V(PCOLR0));
    a.i("LDY", Imm, V(0x10));
    a.i("CPY", Zp, V(PCOLR0));
    a.i("BCC", Rel, L("dli_out"));
    a.i("STY", Zp, V(PCOLR0));
    a.here("dli_out");
    a.i("JMP", Abs, V(0xfcb2));

    // ---- $FFFA: hardware vectors ----
    a.org(0xfffa);
    a.bytes(&0xfca2u16.to_le_bytes());
    a.bytes(&0xfd23u16.to_le_bytes());
    a.bytes(&0xfc00u16.to_le_bytes());
    a.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(img: &[u8; 0x800], at: u16) -> u16 {
        let o = (at - 0xf800) as usize;
        u16::from_le_bytes([img[o], img[o + 1]])
    }

    #[test]
    fn the_hardware_vectors_point_at_the_dispatchers() {
        let img = image();
        assert_eq!(word(&img, 0xfffa), 0xfca2);
        assert_eq!(word(&img, 0xfffc), 0xfd23);
        assert_eq!(word(&img, 0xfffe), 0xfc00);
    }

    #[test]
    fn the_census_entry_points_hold_the_instructions_titles_expect() {
        let img = image();
        let at = |a: u16| img[(a - 0xf800) as usize];
        assert_eq!(at(0xfc00), 0x6c, "JMP ($0200)");
        assert_eq!(word(&img, 0xfc01), 0x0200);
        assert_eq!(at(0xfcb2), 0x68, "PLA, the exit tail");
        assert_eq!(at(0xfcb7), 0x40, "RTI");
        assert_eq!(at(0xfcbd), 0xe6, "INC $02, where Blueprint enters");
        assert_eq!(at(0xfd10), 0x6c, "JMP ($020A) after the keypad decode");
        assert_eq!(word(&img, 0xfd11), 0x020a);
        assert_eq!(at(0xfd23), 0x78, "SEI, reset");
    }

    #[test]
    fn the_default_vector_table_points_into_this_image() {
        let img = image();
        let targets: Vec<u16> = (0..6).map(|k| word(&img, 0xfe95 + 2 * k)).collect();
        assert_eq!(targets, vec![0xfc03, 0xfcb8, 0xfcb2, 0xfea1, 0xfd02, 0xfcb2]);
    }

    #[test]
    fn the_keypad_table_maps_each_scan_code_to_its_key() {
        use crate::pokey::Key;
        let keys = [
            (Key::Digit(0), 0x00),
            (Key::Digit(1), 0x01),
            (Key::Digit(5), 0x05),
            (Key::Digit(9), 0x09),
            (Key::Star, 0x0a),
            (Key::Hash, 0x0b),
            (Key::Start, 0x0c),
            (Key::Pause, 0x0d),
            (Key::Reset, 0x0e),
        ];
        for (k, want) in keys {
            assert_eq!(KEY_TABLE[k.code() as usize], want, "{k:?}");
        }
    }
}
