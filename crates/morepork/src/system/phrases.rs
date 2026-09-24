//! Semantic query phrases, keyed by the system id a header states.

use super::{gb, ExactPhrase, Phrases};
use crate::query::Condition;

static GB: Phrases = Phrases {
    exact: gb::EXACT_PHRASES,
    numbered: gb::NUMBERED_PHRASES,
};

/// The TI VDP's active display is lines 0-191; the frame interrupt rises
/// entering line 192 (0xC0).
static TI_VDP_EXACT: &[ExactPhrase] = &[("vblank starts", || Condition::FieldChangesTo {
    field: "vdp_line".into(),
    value: "0xc0".into(),
})];

static TI_VDP: Phrases = Phrases {
    exact: TI_VDP_EXACT,
    numbered: &[],
};

/// NTSC vblank begins on scanline 241 (0xF1).
static NES_EXACT: &[ExactPhrase] = &[("vblank starts", || Condition::FieldChangesTo {
    field: "line".into(),
    value: "0xf1".into(),
})];

static NES: Phrases = Phrases {
    exact: NES_EXACT,
    numbered: &[],
};

static NONE: Phrases = Phrases {
    exact: &[],
    numbered: &[],
};

/// The phrases for a system id; none for a system without any.
pub(super) fn for_system(id: &str) -> &'static Phrases {
    match id {
        "dmg" | "cgb" => &GB,
        "sg1000" | "colecovision" | "msx1" => &TI_VDP,
        "nes" => &NES,
        _ => &NONE,
    }
}
