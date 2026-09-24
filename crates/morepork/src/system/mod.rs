//! What a trace's system means to the tools.
//!
//! A trace header states its system: the id, the ISA, the instruction-address
//! column, the diff-alignment hint and a typed declaration of every column.
//! The machine-state vocabulary is missingno's — each system's state schema —
//! and reaches this crate only through headers. What morepork keeps is its
//! own trace-level vocabulary: the flag bits of each ISA for `flag …` queries
//! and the semantic query phrases of each system. See `docs/multi-system.md`.

use crate::error::{Error, Result};
use crate::header::TraceHeader;
use crate::query::Condition;

pub mod gb;
mod isa;
mod phrases;

pub use isa::{isa, ISAS, MOS6502, SM83, Z80};

/// A named CPU flag: which field holds it and at which bit. The first name
/// is canonical (single letter); the rest are accepted aliases.
pub struct FlagDef {
    pub names: &'static [&'static str],
    pub field: &'static str,
    pub bit: u8,
}

/// An instruction-set architecture's flag vocabulary, for `flag …` queries.
pub struct Isa {
    /// Identifier stored in the trace header (`"sm83"`, `"6502"`, `"z80"`).
    pub id: &'static str,

    /// Flag vocabulary in display order (high bit first).
    pub flags: &'static [FlagDef],
}

/// A semantic phrase that is exactly one fixed string (`"lcd on"`),
/// desugaring to a generic [`Condition`].
pub type ExactPhrase = (&'static str, fn() -> Condition);

/// A semantic phrase of the form `<prefix><number>` (`"interrupt 2"`),
/// with an inclusive maximum for the number.
pub type NumberedPhrase = (&'static str, u8, fn(u8) -> Condition);

/// A system's semantic query phrases.
pub struct Phrases {
    pub exact: &'static [ExactPhrase],
    pub numbered: &'static [NumberedPhrase],
}

/// A trace's system as its header states it, plus morepork's own vocabulary
/// for it.
pub struct SystemView<'h> {
    pub id: &'h str,
    pub isa: &'static Isa,
    /// Diff-alignment hint: the address every trace of this system reaches
    /// at program entry, and the address of the entry's second instruction.
    pub entry_addrs: Option<(u16, u16)>,
    /// Empty for a system morepork has no phrases for.
    pub phrases: &'static Phrases,
}

impl<'h> SystemView<'h> {
    /// Read the system from a header. A header that declares no columns, or
    /// names an ISA morepork has no flag table for, is rejected.
    pub fn of(header: &'h TraceHeader) -> Result<Self> {
        if header.field_defs.is_empty() {
            return Err(Error::InvalidHeader(
                "header declares no field_defs; a legacy trace is typed by `morepork convert`"
                    .into(),
            ));
        }
        let isa = isa(&header.isa).ok_or_else(|| {
            let known: Vec<&str> = ISAS.iter().map(|i| i.id).collect();
            Error::InvalidHeader(format!(
                "unknown ISA '{}': expected one of {}",
                header.isa,
                known.join(", ")
            ))
        })?;
        Ok(Self {
            id: &header.system,
            isa,
            entry_addrs: header.entry_addrs,
            phrases: phrases::for_system(&header.system),
        })
    }
}
