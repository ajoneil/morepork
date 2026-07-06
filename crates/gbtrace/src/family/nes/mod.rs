//! The NES family. NTSC/PAL and clones are models within this family, not
//! separate families.
//!
//! The catalogue starts with the state missingno-nes exposes publicly
//! (CPU registers, PPU control/mask and beam position) and grows with its
//! tracer; adapters can carry anything else as extension fields.

use super::{Family, FlagDef, field};
use crate::profile::{FieldDef, FieldType, Layer, SubsystemDef};

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
            field!("cycles", u8),
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

pub static NES: Family = Family {
    id: "nes",
    subsystems: SUBSYSTEMS,
    flags: FLAGS,
    exact_phrases: &[],
    numbered_phrases: &[],
    disassemble: Some(disasm::disassemble),
    // The reset vector is ROM-dependent, so there is no fixed entry
    // address; diff falls back to first-common-address alignment.
    entry_addrs: None,
};
