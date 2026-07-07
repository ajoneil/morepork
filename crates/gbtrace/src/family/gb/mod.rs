//! The Game Boy family: field catalogue, query vocabulary, SM83
//! disassembler, and diff-alignment hints. DMG and CGB are models within
//! this family (the header's free-form `model` string), not separate
//! families.

use super::{ExactPhrase, Family, FlagDef, LabelledPhrase, NumberedPhrase};
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

static EXACT_PHRASES: &[ExactPhrase] = &[
    ("lcd on", || Condition::BitTransition { field: "lcdc".into(), bit: 7, to: true }),
    ("lcd off", || Condition::BitTransition { field: "lcdc".into(), bit: 7, to: false }),
    ("timer overflow", || Condition::FieldWraps { field: "tima".into() }),
];

static NUMBERED_PHRASES: &[NumberedPhrase] = &[
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

static LABELLED_PHRASES: &[LabelledPhrase] = &[
    LabelledPhrase { group: "PPU", label: "HBlank", query: "ppu enters mode 0", needs: "stat" },
    LabelledPhrase { group: "PPU", label: "VBlank", query: "ppu enters mode 1", needs: "stat" },
    LabelledPhrase { group: "PPU", label: "OAM Scan", query: "ppu enters mode 2", needs: "stat" },
    LabelledPhrase { group: "PPU", label: "Drawing", query: "ppu enters mode 3", needs: "stat" },
    LabelledPhrase { group: "PPU", label: "LCD On", query: "lcd on", needs: "lcdc" },
    LabelledPhrase { group: "PPU", label: "LCD Off", query: "lcd off", needs: "lcdc" },
    LabelledPhrase { group: "IRQ", label: "VBlank", query: "interrupt 0", needs: "if_" },
    LabelledPhrase { group: "IRQ", label: "STAT", query: "interrupt 1", needs: "if_" },
    LabelledPhrase { group: "IRQ", label: "Timer", query: "interrupt 2", needs: "if_" },
    LabelledPhrase { group: "IRQ", label: "Serial", query: "interrupt 3", needs: "if_" },
    LabelledPhrase { group: "IRQ", label: "Joypad", query: "interrupt 4", needs: "if_" },
    LabelledPhrase { group: "Timer", label: "Overflow", query: "timer overflow", needs: "tima" },
];

pub static GB: Family = Family {
    id: "gb",
    subsystems: catalogue::SUBSYSTEMS,
    flags: FLAGS,
    exact_phrases: EXACT_PHRASES,
    numbered_phrases: NUMBERED_PHRASES,
    labelled_phrases: LABELLED_PHRASES,
    disassemble: Some(disasm::disassemble),
    entry_addrs: Some((0x0100, 0x0101)),
};
