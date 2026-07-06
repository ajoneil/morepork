//! NMOS 6502 disassembler — all 256 opcodes, documented and illegal.
//!
//! The decode table is generated from missingno-6502's `DECODE` table
//! (its `src/decode.rs`) and formats mnemonics the same way its
//! `disasm.rs` does, so the two stay comparable output-for-output.

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

/// Map a CPU address to a ROM-file offset. iNES files (magic "NES\x1a")
/// map PRG-ROM at 0x8000, mirrored when a single 16 KiB bank (NROM-128);
/// anything else indexes the slice directly.
fn rom_offset(rom: &[u8], addr: u16) -> Option<usize> {
    if rom.len() >= 16 && &rom[0..4] == b"NES\x1a" {
        let trainer = if rom[6] & 0x04 != 0 { 512 } else { 0 };
        let prg_start = 16 + trainer;
        let prg_size = rom[4] as usize * 16384;
        if addr < 0x8000 || prg_size == 0 {
            return None;
        }
        let off = prg_start + (addr as usize - 0x8000) % prg_size;
        (off < rom.len()).then_some(off)
    } else {
        let off = addr as usize;
        (off < rom.len()).then_some(off)
    }
}

/// Disassemble the instruction at CPU address `addr`.
/// Returns (mnemonic, instruction length in bytes).
pub fn disassemble(rom: &[u8], addr: u16) -> (String, u8) {
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

#[cfg(test)]
mod tests {
    use super::disassemble;

    #[test]
    fn formats_each_addressing_mode() {
        // Raw (headerless) slice indexes directly.
        let rom = [
            0xa9, 0x42,       // 0: lda #$42
            0x8d, 0x00, 0x20, // 2: sta $2000
            0x10, 0xfe,       // 5: bpl $0005 (-2 from following addr 7)
            0x6c, 0x34, 0x12, // 7: jmp ($1234)
            0x96, 0x10,       // 10: stx $10,y
            0x03, 0x20,       // 12: slo ($20,x)
            0x0a,             // 14: asl a
            0x00,             // 15: brk
        ];
        let cases = [
            (0, "lda #$42", 2),
            (2, "sta $2000", 3),
            (5, "bpl $0005", 2),
            (7, "jmp ($1234)", 3),
            (10, "stx $10,y", 2),
            (12, "slo ($20,x)", 2),
            (14, "asl a", 1),
            (15, "brk", 1),
        ];
        for (addr, mnemonic, len) in cases {
            assert_eq!(disassemble(&rom, addr), (mnemonic.to_string(), len));
        }
    }

    #[test]
    fn maps_ines_prg_with_mirroring() {
        // NROM-128: one 16 KiB PRG bank, mirrored at 0x8000 and 0xC000.
        let mut rom = vec![0u8; 16 + 16384];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 1; // one PRG bank
        rom[16] = 0xea; // nop at CPU 0x8000
        rom[16 + 0x123] = 0xa9; // lda # at CPU 0x8123
        rom[16 + 0x124] = 0x7f;
        assert_eq!(disassemble(&rom, 0x8000).0, "nop");
        assert_eq!(disassemble(&rom, 0x8123).0, "lda #$7f");
        // Mirror: 0xC123 hits the same bytes.
        assert_eq!(disassemble(&rom, 0xc123).0, "lda #$7f");
        // Below PRG space there is nothing to disassemble.
        assert_eq!(disassemble(&rom, 0x4000).0, "??");
    }
}
