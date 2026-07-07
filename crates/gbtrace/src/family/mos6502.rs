//! Shared NMOS 6502 disassembler core — all 256 opcodes, documented and
//! illegal. Used by every family built on the 6502 (NES's 2A03, the
//! VCS's 6507); each family supplies its own CPU-address-to-ROM-offset
//! mapping.
//!
//! The decode table is generated from missingno-6502's `DECODE` table
//! (its `src/decode.rs`) and formats mnemonics the same way its
//! `disasm.rs` does, so the two stay comparable output-for-output.

use crate::profile::{FieldDef, FieldType, Layer, SubsystemDef};
use super::{FlagDef, field};

/// The 6502 register file, shared by every family carrying this core.
pub static CPU: SubsystemDef = SubsystemDef {
    name: "cpu",
    layers: &[
        (Layer::Registers, &[
            field!("pc", u16),
            field!("a", u8),
            field!("x", u8),
            field!("y", u8),
            field!("s", u8),
            field!("p", u8, dict),
        ]),
        (Layer::Internal, &[
            field!("rdy", bool),
        ]),
        (Layer::Timing, &[
            // u16: one instruction can stall the CPU far past a u8 delta
            // (NES OAM DMA freezes it 513+ cycles; VCS WSYNC parks it
            // for the rest of the scanline).
            field!("cycles", u16),
        ]),
    ],
};

/// 6502 status flags in P. B (bit 4) only exists in pushed copies of P,
/// so it is not part of the vocabulary.
pub static FLAGS: &[FlagDef] = &[
    FlagDef { names: &["n", "negative"], field: "p", bit: 7 },
    FlagDef { names: &["v", "overflow"], field: "p", bit: 6 },
    FlagDef { names: &["d", "decimal"], field: "p", bit: 3 },
    FlagDef { names: &["i", "interrupt"], field: "p", bit: 2 },
    FlagDef { names: &["z", "zero"], field: "p", bit: 1 },
    FlagDef { names: &["c", "carry"], field: "p", bit: 0 },
];

#[derive(Clone, Copy)]
enum Mode {
    Implied,
    Accumulator,
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    IndirectX,
    IndirectY,
    Relative,
    Indirect,
}

use Mode::*;

#[rustfmt::skip]
static OPS: [(&str, Mode); 256] = [
    // 0x00
    ("brk", Implied), ("ora", IndirectX), ("jam", Implied), ("slo", IndirectX),
    ("nop", ZeroPage), ("ora", ZeroPage), ("asl", ZeroPage), ("slo", ZeroPage),
    ("php", Implied), ("ora", Immediate), ("asl", Accumulator), ("anc", Immediate),
    ("nop", Absolute), ("ora", Absolute), ("asl", Absolute), ("slo", Absolute),
    // 0x10
    ("bpl", Relative), ("ora", IndirectY), ("jam", Implied), ("slo", IndirectY),
    ("nop", ZeroPageX), ("ora", ZeroPageX), ("asl", ZeroPageX), ("slo", ZeroPageX),
    ("clc", Implied), ("ora", AbsoluteY), ("nop", Implied), ("slo", AbsoluteY),
    ("nop", AbsoluteX), ("ora", AbsoluteX), ("asl", AbsoluteX), ("slo", AbsoluteX),
    // 0x20
    ("jsr", Absolute), ("and", IndirectX), ("jam", Implied), ("rla", IndirectX),
    ("bit", ZeroPage), ("and", ZeroPage), ("rol", ZeroPage), ("rla", ZeroPage),
    ("plp", Implied), ("and", Immediate), ("rol", Accumulator), ("anc", Immediate),
    ("bit", Absolute), ("and", Absolute), ("rol", Absolute), ("rla", Absolute),
    // 0x30
    ("bmi", Relative), ("and", IndirectY), ("jam", Implied), ("rla", IndirectY),
    ("nop", ZeroPageX), ("and", ZeroPageX), ("rol", ZeroPageX), ("rla", ZeroPageX),
    ("sec", Implied), ("and", AbsoluteY), ("nop", Implied), ("rla", AbsoluteY),
    ("nop", AbsoluteX), ("and", AbsoluteX), ("rol", AbsoluteX), ("rla", AbsoluteX),
    // 0x40
    ("rti", Implied), ("eor", IndirectX), ("jam", Implied), ("sre", IndirectX),
    ("nop", ZeroPage), ("eor", ZeroPage), ("lsr", ZeroPage), ("sre", ZeroPage),
    ("pha", Implied), ("eor", Immediate), ("lsr", Accumulator), ("alr", Immediate),
    ("jmp", Absolute), ("eor", Absolute), ("lsr", Absolute), ("sre", Absolute),
    // 0x50
    ("bvc", Relative), ("eor", IndirectY), ("jam", Implied), ("sre", IndirectY),
    ("nop", ZeroPageX), ("eor", ZeroPageX), ("lsr", ZeroPageX), ("sre", ZeroPageX),
    ("cli", Implied), ("eor", AbsoluteY), ("nop", Implied), ("sre", AbsoluteY),
    ("nop", AbsoluteX), ("eor", AbsoluteX), ("lsr", AbsoluteX), ("sre", AbsoluteX),
    // 0x60
    ("rts", Implied), ("adc", IndirectX), ("jam", Implied), ("rra", IndirectX),
    ("nop", ZeroPage), ("adc", ZeroPage), ("ror", ZeroPage), ("rra", ZeroPage),
    ("pla", Implied), ("adc", Immediate), ("ror", Accumulator), ("arr", Immediate),
    ("jmp", Indirect), ("adc", Absolute), ("ror", Absolute), ("rra", Absolute),
    // 0x70
    ("bvs", Relative), ("adc", IndirectY), ("jam", Implied), ("rra", IndirectY),
    ("nop", ZeroPageX), ("adc", ZeroPageX), ("ror", ZeroPageX), ("rra", ZeroPageX),
    ("sei", Implied), ("adc", AbsoluteY), ("nop", Implied), ("rra", AbsoluteY),
    ("nop", AbsoluteX), ("adc", AbsoluteX), ("ror", AbsoluteX), ("rra", AbsoluteX),
    // 0x80
    ("nop", Immediate), ("sta", IndirectX), ("nop", Immediate), ("sax", IndirectX),
    ("sty", ZeroPage), ("sta", ZeroPage), ("stx", ZeroPage), ("sax", ZeroPage),
    ("dey", Implied), ("nop", Immediate), ("txa", Implied), ("ane", Immediate),
    ("sty", Absolute), ("sta", Absolute), ("stx", Absolute), ("sax", Absolute),
    // 0x90
    ("bcc", Relative), ("sta", IndirectY), ("jam", Implied), ("sha", IndirectY),
    ("sty", ZeroPageX), ("sta", ZeroPageX), ("stx", ZeroPageY), ("sax", ZeroPageY),
    ("tya", Implied), ("sta", AbsoluteY), ("txs", Implied), ("tas", AbsoluteY),
    ("shy", AbsoluteX), ("sta", AbsoluteX), ("shx", AbsoluteY), ("sha", AbsoluteY),
    // 0xA0
    ("ldy", Immediate), ("lda", IndirectX), ("ldx", Immediate), ("lax", IndirectX),
    ("ldy", ZeroPage), ("lda", ZeroPage), ("ldx", ZeroPage), ("lax", ZeroPage),
    ("tay", Implied), ("lda", Immediate), ("tax", Implied), ("lxa", Immediate),
    ("ldy", Absolute), ("lda", Absolute), ("ldx", Absolute), ("lax", Absolute),
    // 0xB0
    ("bcs", Relative), ("lda", IndirectY), ("jam", Implied), ("lax", IndirectY),
    ("ldy", ZeroPageX), ("lda", ZeroPageX), ("ldx", ZeroPageY), ("lax", ZeroPageY),
    ("clv", Implied), ("lda", AbsoluteY), ("tsx", Implied), ("las", AbsoluteY),
    ("ldy", AbsoluteX), ("lda", AbsoluteX), ("ldx", AbsoluteY), ("lax", AbsoluteY),
    // 0xC0
    ("cpy", Immediate), ("cmp", IndirectX), ("nop", Immediate), ("dcp", IndirectX),
    ("cpy", ZeroPage), ("cmp", ZeroPage), ("dec", ZeroPage), ("dcp", ZeroPage),
    ("iny", Implied), ("cmp", Immediate), ("dex", Implied), ("sbx", Immediate),
    ("cpy", Absolute), ("cmp", Absolute), ("dec", Absolute), ("dcp", Absolute),
    // 0xD0
    ("bne", Relative), ("cmp", IndirectY), ("jam", Implied), ("dcp", IndirectY),
    ("nop", ZeroPageX), ("cmp", ZeroPageX), ("dec", ZeroPageX), ("dcp", ZeroPageX),
    ("cld", Implied), ("cmp", AbsoluteY), ("nop", Implied), ("dcp", AbsoluteY),
    ("nop", AbsoluteX), ("cmp", AbsoluteX), ("dec", AbsoluteX), ("dcp", AbsoluteX),
    // 0xE0
    ("cpx", Immediate), ("sbc", IndirectX), ("nop", Immediate), ("isc", IndirectX),
    ("cpx", ZeroPage), ("sbc", ZeroPage), ("inc", ZeroPage), ("isc", ZeroPage),
    ("inx", Implied), ("sbc", Immediate), ("nop", Implied), ("sbc", Immediate),
    ("cpx", Absolute), ("sbc", Absolute), ("inc", Absolute), ("isc", Absolute),
    // 0xF0
    ("beq", Relative), ("sbc", IndirectY), ("jam", Implied), ("isc", IndirectY),
    ("nop", ZeroPageX), ("sbc", ZeroPageX), ("inc", ZeroPageX), ("isc", ZeroPageX),
    ("sed", Implied), ("sbc", AbsoluteY), ("nop", Implied), ("isc", AbsoluteY),
    ("nop", AbsoluteX), ("sbc", AbsoluteX), ("inc", AbsoluteX), ("isc", AbsoluteX),
];

fn length(mode: Mode) -> u8 {
    match mode {
        Implied | Accumulator => 1,
        Immediate | ZeroPage | ZeroPageX | ZeroPageY | IndirectX | IndirectY
        | Relative => 2,
        Absolute | AbsoluteX | AbsoluteY | Indirect => 3,
    }
}


/// Disassemble the instruction at CPU address `addr`, reading bytes
/// through the family's `rom_offset` mapping (CPU address -> ROM-file
/// offset, `None` when the address is not backed by ROM).
pub fn disassemble(
    rom: &[u8],
    addr: u16,
    rom_offset: fn(&[u8], u16) -> Option<usize>,
) -> (String, u8) {
    let byte_at = |a: u16| rom_offset(rom, a).map(|o| rom[o]).unwrap_or(0);
    let opcode = match rom_offset(rom, addr) {
        Some(o) => rom[o],
        None => return ("??".to_string(), 1),
    };
    let (name, mode) = OPS[opcode as usize];
    let length = length(mode);
    let operand_byte = byte_at(addr.wrapping_add(1));
    let operand_word =
        u16::from_le_bytes([operand_byte, byte_at(addr.wrapping_add(2))]);
    let mnemonic = match mode {
        Implied => name.to_string(),
        Accumulator => format!("{name} a"),
        Immediate => format!("{name} #${operand_byte:02x}"),
        ZeroPage => format!("{name} ${operand_byte:02x}"),
        ZeroPageX => format!("{name} ${operand_byte:02x},x"),
        ZeroPageY => format!("{name} ${operand_byte:02x},y"),
        Absolute => format!("{name} ${operand_word:04x}"),
        AbsoluteX => format!("{name} ${operand_word:04x},x"),
        AbsoluteY => format!("{name} ${operand_word:04x},y"),
        IndirectX => format!("{name} (${operand_byte:02x},x)"),
        IndirectY => format!("{name} (${operand_byte:02x}),y"),
        Indirect => format!("{name} (${operand_word:04x})"),
        Relative => {
            let target = addr
                .wrapping_add(2)
                .wrapping_add(operand_byte as i8 as u16);
            format!("{name} ${target:04x}")
        }
    };
    (mnemonic, length)
}
