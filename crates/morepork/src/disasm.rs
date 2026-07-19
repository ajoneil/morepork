//! Decode-for-display, driven by the shared `missingno_core` instruction-set
//! vocabulary rather than a decoder of morepork's own. A caller that can name
//! the trace's ISA (via [`missingno_core::isa::InstructionSet`]) disassembles
//! the ROM image through the same front end the debugger uses, so the two
//! never drift.

use missingno_core::isa::{Instruction, InstructionSet};

/// Decode the instruction at `address`, reading up to the ISA's maximum
/// instruction length from `rom` (a flat program image indexed by address,
/// wrapped to the ISA's address width).
pub fn decode_at(isa: &dyn InstructionSet, rom: &[u8], address: u32) -> Instruction {
    let mask = isa.address_mask();
    let start = (address & mask) as usize;
    let end = (start + isa.max_len()).min(rom.len());
    let bytes = rom.get(start..end).unwrap_or(&[]);
    isa.decode(address & mask, bytes)
}

/// Disassemble `count` consecutive instructions from `address`, each row the
/// address and decoded mnemonic. Every row advances by the decoded length.
pub fn disassemble_rows(
    isa: &dyn InstructionSet,
    rom: &[u8],
    address: u32,
    count: usize,
) -> Vec<(u32, String)> {
    let mask = isa.address_mask();
    let mut pc = address & mask;
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let instruction = decode_at(isa, rom, pc);
        let advance = instruction.length.max(1) as u32;
        rows.push((pc, instruction.mnemonic));
        pc = pc.wrapping_add(advance) & mask;
    }
    rows
}
