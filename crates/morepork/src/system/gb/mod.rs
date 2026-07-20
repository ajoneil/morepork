//! The Game Boy: field catalogue, query vocabulary, frame reconstruction, and
//! diff-alignment hints, shared by the [`DMG`] and [`CGB`] systems. They are
//! distinct systems on the shared `sm83` [`Isa`](super::Isa): the CGB adds
//! colour-palette, double-speed, bank, and HDMA state (the `cgb` subsystem)
//! but renders and queries identically.

use super::{ExactPhrase, FlagDef, NumberedPhrase, System};
use crate::query::Condition;

pub mod catalogue;
pub mod framebuffer;
pub mod vram;

pub static FLAGS: &[FlagDef] = &[
    FlagDef {
        names: &["z", "zero"],
        field: "f",
        bit: 7,
    },
    FlagDef {
        names: &["n", "sub", "subtract"],
        field: "f",
        bit: 6,
    },
    FlagDef {
        names: &["h", "half", "halfcarry"],
        field: "f",
        bit: 5,
    },
    FlagDef {
        names: &["c", "carry"],
        field: "f",
        bit: 4,
    },
];

static EXACT_PHRASES: &[ExactPhrase] = &[
    ("lcd on", || Condition::BitTransition {
        field: "lcdc".into(),
        bit: 7,
        to: true,
    }),
    ("lcd off", || Condition::BitTransition {
        field: "lcdc".into(),
        bit: 7,
        to: false,
    }),
    ("timer overflow", || Condition::FieldWraps {
        field: "tima".into(),
    }),
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

/// The original Game Boy (DMG). 2-bit greyscale; the base catalogue.
pub static DMG: System = System {
    id: "dmg",
    isa: &super::SM83,
    subsystems: catalogue::SUBSYSTEMS_DMG,
    exact_phrases: EXACT_PHRASES,
    numbered_phrases: NUMBERED_PHRASES,
    entry_addrs: Some((0x0100, 0x0101)),
};

/// The Game Boy Color (CGB): the DMG plus CGB-only state (colour palettes,
/// KEY1 double-speed, VRAM/WRAM banks, HDMA — the `cgb` subsystem). Shares
/// the SM83 ISA, frame reconstruction, and query phrases with [`DMG`];
/// `pix_format` (rgb555 vs shade2) is set by the adapter, not the system.
pub static CGB: System = System {
    id: "cgb",
    isa: &super::SM83,
    subsystems: catalogue::SUBSYSTEMS_CGB,
    exact_phrases: EXACT_PHRASES,
    numbered_phrases: NUMBERED_PHRASES,
    entry_addrs: Some((0x0100, 0x0101)),
};
