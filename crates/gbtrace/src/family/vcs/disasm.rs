//! VCS disassembly: the shared 6502 core plus the 6507's cartridge map.

use crate::family::mos6502;

/// Map a CPU address to a ROM-file offset. The 6507 has 13 address
/// lines; A12 selects the cartridge (a 4 KiB window at $1000-$1FFF,
/// conventionally written $F000-$FFFF), and smaller ROMs mirror up.
/// Below A12 sit the TIA and RIOT — nothing to disassemble.
fn rom_offset(rom: &[u8], addr: u16) -> Option<usize> {
    if rom.is_empty() {
        return None;
    }
    let addr = addr & 0x1FFF;
    if addr & 0x1000 == 0 {
        return None;
    }
    Some((addr as usize & 0xFFF) % rom.len())
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
    fn maps_the_6507_cartridge_window() {
        // 4 KiB cartridge; the CPU sees it at $F000 (and every A12=1 mirror).
        let mut rom = vec![0u8; 0x1000];
        rom[0] = 0xa2; // ldx #$00 at $F000
        rom[1] = 0x00;
        rom[0x123] = 0x85; // sta $02 at $F123
        rom[0x124] = 0x02;
        assert_eq!(disassemble(&rom, 0xF000).0, "ldx #$00");
        assert_eq!(disassemble(&rom, 0xF123).0, "sta $02");
        // Any A12=1 mirror reads the same bytes ($1123 = $F123 & 0x1FFF).
        assert_eq!(disassemble(&rom, 0x1123).0, "sta $02");
        // TIA/RIOT space is not ROM.
        assert_eq!(disassemble(&rom, 0x0080).0, "??");
    }

    #[test]
    fn mirrors_2k_roms() {
        let mut rom = vec![0u8; 0x800];
        rom[0x7FF] = 0xea; // nop at the top of the 2 KiB image
        assert_eq!(disassemble(&rom, 0xF7FF).0, "nop");
        // The upper half of the 4 KiB window mirrors the same 2 KiB.
        assert_eq!(disassemble(&rom, 0xFFFF).0, "nop");
    }
}
