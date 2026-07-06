//! The Game Boy family: field catalogue, query vocabulary, SM83
//! disassembler, and diff-alignment hints. DMG and CGB are models within
//! this family (the header's free-form `model` string), not separate
//! families.

use super::{Family, FlagDef};
use crate::query::Condition;

pub mod catalogue;
pub mod disasm;
pub mod framebuffer;
pub mod snapshot;
pub mod vram;

static FLAGS: &[FlagDef] = &[
    FlagDef { names: &["z", "zero"], field: "f", bit: 7 },
    FlagDef { names: &["n", "sub", "subtract"], field: "f", bit: 6 },
    FlagDef { names: &["h", "half", "halfcarry"], field: "f", bit: 5 },
    FlagDef { names: &["c", "carry"], field: "f", bit: 4 },
];

static EXACT_PHRASES: &[(&str, fn() -> Condition)] = &[
    ("lcd on", || Condition::BitTransition { field: "lcdc".into(), bit: 7, to: true }),
    ("lcd off", || Condition::BitTransition { field: "lcdc".into(), bit: 7, to: false }),
    ("timer overflow", || Condition::FieldWraps { field: "tima".into() }),
];

static NUMBERED_PHRASES: &[(&str, u8, fn(u8) -> Condition)] = &[
    ("ppu enters mode ", 3, |mode| Condition::MaskedChangesTo {
        field: "stat".into(),
        mask: 0x03,
        value: mode as u64,
    }),
    ("interrupt ", 4, |bit| Condition::BitTransition {
        field: "if_".into(),
        bit,
        to: true,
    }),
];

pub static GB: Family = Family {
    id: "gb",
    subsystems: catalogue::SUBSYSTEMS,
    flags: FLAGS,
    exact_phrases: EXACT_PHRASES,
    numbered_phrases: NUMBERED_PHRASES,
    disassemble: Some(disasm::disassemble),
    entry_addrs: Some((0x0100, 0x0101)),
};
