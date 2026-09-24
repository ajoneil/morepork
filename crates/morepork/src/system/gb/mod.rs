//! The Game Boy's trace-level vocabulary: its query phrases and frame
//! reconstruction from the `pix` stream, shared by the `dmg` and `cgb`
//! systems.

use super::{ExactPhrase, NumberedPhrase};
use crate::query::Condition;

pub mod framebuffer;
pub mod vram;

pub(super) static EXACT_PHRASES: &[ExactPhrase] = &[
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

pub(super) static NUMBERED_PHRASES: &[NumberedPhrase] = &[
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
