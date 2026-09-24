//! The flag vocabulary of each ISA a trace header can name.

use super::{FlagDef, Isa};

/// The Sharp SM83 (Game Boy): flags in F's high nibble.
static SM83_FLAGS: &[FlagDef] = &[
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

/// 6502 status flags in P. B (bit 4) only exists in pushed copies of P,
/// so it is not part of the vocabulary.
static MOS6502_FLAGS: &[FlagDef] = &[
    FlagDef {
        names: &["n", "negative"],
        field: "p",
        bit: 7,
    },
    FlagDef {
        names: &["v", "overflow"],
        field: "p",
        bit: 6,
    },
    FlagDef {
        names: &["d", "decimal"],
        field: "p",
        bit: 3,
    },
    FlagDef {
        names: &["i", "interrupt"],
        field: "p",
        bit: 2,
    },
    FlagDef {
        names: &["z", "zero"],
        field: "p",
        bit: 1,
    },
    FlagDef {
        names: &["c", "carry"],
        field: "p",
        bit: 0,
    },
];

/// Z80 status flags in F, high bit first. X and Y are the undocumented
/// copy-of-result bits 3 and 5 — part of the vocabulary because exercising
/// them is exactly what Z80 test ROMs do.
static Z80_FLAGS: &[FlagDef] = &[
    FlagDef {
        names: &["s", "sign"],
        field: "f",
        bit: 7,
    },
    FlagDef {
        names: &["z", "zero"],
        field: "f",
        bit: 6,
    },
    FlagDef {
        names: &["y"],
        field: "f",
        bit: 5,
    },
    FlagDef {
        names: &["h", "half"],
        field: "f",
        bit: 4,
    },
    FlagDef {
        names: &["x"],
        field: "f",
        bit: 3,
    },
    FlagDef {
        names: &["p", "pv", "parity", "overflow"],
        field: "f",
        bit: 2,
    },
    FlagDef {
        names: &["n", "sub", "subtract"],
        field: "f",
        bit: 1,
    },
    FlagDef {
        names: &["c", "carry"],
        field: "f",
        bit: 0,
    },
];

pub static SM83: Isa = Isa {
    id: "sm83",
    flags: SM83_FLAGS,
};
pub static MOS6502: Isa = Isa {
    id: "6502",
    flags: MOS6502_FLAGS,
};
pub static Z80: Isa = Isa {
    id: "z80",
    flags: Z80_FLAGS,
};

/// Every ISA a header may name.
pub static ISAS: &[&Isa] = &[&SM83, &MOS6502, &Z80];

/// Look up an ISA by id.
pub fn isa(id: &str) -> Option<&'static Isa> {
    ISAS.iter().copied().find(|i| i.id == id)
}
