// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Prodigy75000

//! A 6502 disassembler for the dev harnesses: when every counter says fine,
//! ask what the CPU is executing. Undocumented opcodes get their common names
//! so a trace through cartridge code that uses them stays readable.

/// Addressing mode, which also fixes the instruction length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Imp,
    Acc,
    Imm,
    Zp,
    Zpx,
    Zpy,
    Abs,
    Abx,
    Aby,
    Ind,
    Izx,
    Izy,
    Rel,
}

impl Mode {
    pub fn len(self) -> u16 {
        match self {
            Mode::Imp | Mode::Acc => 1,
            Mode::Abs | Mode::Abx | Mode::Aby | Mode::Ind => 3,
            _ => 2,
        }
    }
}

/// Mnemonic and addressing mode for an opcode.
pub fn decode(op: u8) -> (&'static str, Mode) {
    use Mode::*;
    let (a, b, c) = (op >> 5, (op >> 2) & 7, op & 3);
    // The regular part of the matrix first, then the exceptions.
    match op {
        0x00 => return ("BRK", Imp),
        0x20 => return ("JSR", Abs),
        0x40 => return ("RTI", Imp),
        0x60 => return ("RTS", Imp),
        0x4c => return ("JMP", Abs),
        0x6c => return ("JMP", Ind),
        0x08 => return ("PHP", Imp),
        0x28 => return ("PLP", Imp),
        0x48 => return ("PHA", Imp),
        0x68 => return ("PLA", Imp),
        0x88 => return ("DEY", Imp),
        0xa8 => return ("TAY", Imp),
        0xc8 => return ("INY", Imp),
        0xe8 => return ("INX", Imp),
        0x18 => return ("CLC", Imp),
        0x38 => return ("SEC", Imp),
        0x58 => return ("CLI", Imp),
        0x78 => return ("SEI", Imp),
        0x98 => return ("TYA", Imp),
        0xb8 => return ("CLV", Imp),
        0xd8 => return ("CLD", Imp),
        0xf8 => return ("SED", Imp),
        0x8a => return ("TXA", Imp),
        0x9a => return ("TXS", Imp),
        0xaa => return ("TAX", Imp),
        0xba => return ("TSX", Imp),
        0xca => return ("DEX", Imp),
        0xea => return ("NOP", Imp),
        0x0a | 0x2a | 0x4a | 0x6a => {
            return (["ASL", "ROL", "LSR", "ROR"][(op >> 5) as usize], Acc)
        }
        0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xb2 | 0xd2 | 0xf2 => {
            return ("JAM", Imp)
        }
        0x1a | 0x3a | 0x5a | 0x7a | 0xda | 0xfa => return ("NOP", Imp),
        0x80 | 0x82 | 0x89 | 0xc2 | 0xe2 => return ("NOP", Imm),
        0x0b | 0x2b => return ("ANC", Imm),
        0x4b => return ("ALR", Imm),
        0x6b => return ("ARR", Imm),
        0x8b => return ("XAA", Imm),
        0xab => return ("LXA", Imm),
        0xcb => return ("SBX", Imm),
        0xeb => return ("SBC", Imm),
        0x9b => return ("TAS", Aby),
        0xbb => return ("LAS", Aby),
        0x9c => return ("SHY", Abx),
        0x9e => return ("SHX", Aby),
        0x9f => return ("SHA", Aby),
        0x93 => return ("SHA", Izy),
        _ => {}
    }
    if c == 0 && b == 4 {
        return (
            ["BPL", "BMI", "BVC", "BVS", "BCC", "BCS", "BNE", "BEQ"][a as usize],
            Rel,
        );
    }
    let mode = match (c, b) {
        (1, 0) | (3, 0) => Izx,
        (1, 4) | (3, 4) => Izy,
        (_, 1) => Zp,
        (0, 0) | (2, 0) => Imm,
        (1, 2) => Imm,
        (_, 3) => Abs,
        (_, 5) => Zpx,
        (1, 6) | (3, 6) => Aby,
        (_, 7) => Abx,
        _ => Imp,
    };
    // LDX/STX (and LAX/SAX) index with Y where the others use X.
    let mode = match (op, mode) {
        (0x96 | 0xb6 | 0x97 | 0xb7, Zpx) => Zpy,
        (0xbe | 0xbf, Abx) => Aby,
        _ => mode,
    };
    // Column 0 indexed by X is only STY and LDY; the rest are undocumented NOPs.
    if c == 0 && (b == 5 || b == 7) && a != 4 && a != 5 {
        return ("NOP", mode);
    }
    let name = match c {
        0 => ["NOP", "BIT", "NOP", "NOP", "STY", "LDY", "CPY", "CPX"][a as usize],
        1 => ["ORA", "AND", "EOR", "ADC", "STA", "LDA", "CMP", "SBC"][a as usize],
        2 => ["ASL", "ROL", "LSR", "ROR", "STX", "LDX", "DEC", "INC"][a as usize],
        _ => ["SLO", "RLA", "SRE", "RRA", "SAX", "LAX", "DCP", "ISC"][a as usize],
    };
    (name, mode)
}

/// Disassemble one instruction at `pc`, reading bytes through `peek`.
/// Returns the text and the instruction length.
pub fn disasm(pc: u16, mut peek: impl FnMut(u16) -> u8) -> (String, u16) {
    let op = peek(pc);
    let (name, mode) = decode(op);
    let b1 = peek(pc.wrapping_add(1));
    let b2 = peek(pc.wrapping_add(2));
    let w = u16::from_le_bytes([b1, b2]);
    let operand = match mode {
        Mode::Imp => String::new(),
        Mode::Acc => " A".into(),
        Mode::Imm => format!(" #${b1:02X}"),
        Mode::Zp => format!(" ${b1:02X}"),
        Mode::Zpx => format!(" ${b1:02X},X"),
        Mode::Zpy => format!(" ${b1:02X},Y"),
        Mode::Abs => format!(" ${w:04X}"),
        Mode::Abx => format!(" ${w:04X},X"),
        Mode::Aby => format!(" ${w:04X},Y"),
        Mode::Ind => format!(" (${w:04X})"),
        Mode::Izx => format!(" (${b1:02X},X)"),
        Mode::Izy => format!(" (${b1:02X}),Y"),
        Mode::Rel => {
            let t = pc.wrapping_add(2).wrapping_add(b1 as i8 as u16);
            format!(" ${t:04X}")
        }
    };
    (format!("{name}{operand}"), mode.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(bytes: &[u8]) -> String {
        disasm(0x1000, |a| *bytes.get((a - 0x1000) as usize).unwrap_or(&0)).0
    }

    #[test]
    fn known_encodings_decode_to_their_published_text() {
        assert_eq!(one(&[0xa9, 0x12]), "LDA #$12");
        assert_eq!(one(&[0xb6, 0x40]), "LDX $40,Y");
        assert_eq!(one(&[0xbe, 0x00, 0x20]), "LDX $2000,Y");
        assert_eq!(one(&[0x91, 0x80]), "STA ($80),Y");
        assert_eq!(one(&[0x6c, 0xfe, 0xbf]), "JMP ($BFFE)");
        assert_eq!(one(&[0xd0, 0xfe]), "BNE $1000");
        assert_eq!(one(&[0x8d, 0x0a, 0xd4]), "STA $D40A");
        assert_eq!(one(&[0x4a]), "LSR A");
        assert_eq!(one(&[0xa7, 0x10]), "LAX $10");
        assert_eq!(one(&[0x99, 0x00, 0x02]), "STA $0200,Y");
        assert_eq!(one(&[0xe0, 0x05]), "CPX #$05");
        assert_eq!(one(&[0xa2, 0xff]), "LDX #$FF");
    }

    #[test]
    fn lengths_agree_with_the_cpu_for_every_opcode() {
        // The CPU is the authority on how many bytes it consumes. Run each
        // opcode once on a flat bus and compare PC advance, excluding the
        // ones that transfer control.
        use crate::cpu::{Cpu, CpuBus};
        struct Flat([u8; 0x10000]);
        impl CpuBus for Flat {
            fn read(&mut self, a: u16) -> u8 {
                self.0[a as usize]
            }
            fn write(&mut self, a: u16, v: u8) {
                self.0[a as usize] = v;
            }
        }
        for op in 0..=255u8 {
            let (name, mode) = decode(op);
            if matches!(name, "BRK" | "JSR" | "RTI" | "RTS" | "JMP" | "JAM") || mode == Mode::Rel {
                continue;
            }
            let mut bus = Flat([0; 0x10000]);
            bus.0[0x0200] = op;
            let mut cpu = Cpu::new();
            cpu.pc = 0x0200;
            cpu.step(&mut bus);
            assert_eq!(cpu.pc - 0x0200, mode.len(), "opcode {op:02X} {name}");
        }
    }
}
