//! The NES family. NTSC/PAL and clones are models within this family, not
//! separate families.
//!
//! The catalogue starts with the state missingno-nes exposes publicly
//! (CPU registers, PPU control/mask and beam position) and grows with its
//! tracer; adapters can carry anything else as extension fields.

use super::{Family, FlagDef, LabelledPhrase, field};
use crate::profile::{FieldDef, FieldType, Layer, SubsystemDef};
use crate::query::Condition;

pub mod disasm;

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
            // u16: OAM DMA freezes the CPU for 513+ cycles inside one
            // instruction, overflowing a u8 delta.
            field!("cycles", u16),
        ]),
    ],
};

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

pub static SUBSYSTEMS: &[&SubsystemDef] = &[&CPU, &PPU];

/// 6502 status flags in P. B (bit 4) only exists in pushed copies of P,
/// so it is not part of the vocabulary.
static FLAGS: &[FlagDef] = &[
    FlagDef { names: &["n", "negative"], field: "p", bit: 7 },
    FlagDef { names: &["v", "overflow"], field: "p", bit: 6 },
    FlagDef { names: &["d", "decimal"], field: "p", bit: 3 },
    FlagDef { names: &["i", "interrupt"], field: "p", bit: 2 },
    FlagDef { names: &["z", "zero"], field: "p", bit: 1 },
    FlagDef { names: &["c", "carry"], field: "p", bit: 0 },
];

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
    flags: FLAGS,
    exact_phrases: EXACT_PHRASES,
    numbered_phrases: &[],
    labelled_phrases: LABELLED_PHRASES,
    disassemble: Some(disasm::disassemble),
    // The reset vector is ROM-dependent, so there is no fixed entry
    // address; diff falls back to first-common-address alignment.
    entry_addrs: None,
};
