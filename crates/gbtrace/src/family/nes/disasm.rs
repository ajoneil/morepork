//! NES disassembly: the shared 6502 core plus the iNES ROM mapping.

use crate::family::mos6502;

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
    mos6502::disassemble(rom, addr, rom_offset)
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
