//! The NES family. NTSC/PAL and clones are models within this family, not
//! separate families.
//!
//! The catalogue starts with the state missingno-nes exposes publicly
//! (CPU registers, PPU control/mask and beam position) and grows with its
//! tracer; adapters can carry anything else as extension fields.

use super::{Family, LabelledPhrase, field, mos6502};
use crate::profile::{FieldDef, FieldType, Layer, SubsystemDef};
use crate::query::Condition;

pub mod disasm;

pub static PPU: SubsystemDef = SubsystemDef {
    name: "ppu",
    layers: &[
        (Layer::Registers, &[
            field!("control", u8, dict),
            field!("mask", u8, dict),
            field!("line", u16),
            field!("dot", u16),
        ]),
    ],
};

pub static SUBSYSTEMS: &[&SubsystemDef] = &[&mos6502::CPU, &PPU];

/// NTSC vblank begins on scanline 241 (0xF1).
static EXACT_PHRASES: &[(&str, fn() -> Condition)] = &[
    ("vblank starts", || Condition::FieldChangesTo { field: "line".into(), value: "0xf1".into() }),
];

static LABELLED_PHRASES: &[LabelledPhrase] = &[
    LabelledPhrase { group: "PPU", label: "VBlank", query: "vblank starts", needs: "line" },
];

pub static NES: Family = Family {
    id: "nes",
    subsystems: SUBSYSTEMS,
    flags: mos6502::FLAGS,
    exact_phrases: EXACT_PHRASES,
    numbered_phrases: &[],
    labelled_phrases: LABELLED_PHRASES,
    disassemble: Some(disasm::disassemble),
    // The reset vector is ROM-dependent, so there is no fixed entry
    // address; diff falls back to first-common-address alignment.
    entry_addrs: None,
};
