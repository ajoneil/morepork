//! Console-family registry.
//!
//! Self-describing trace headers carry everything a reader needs for
//! info/query/diff/table work; what remains system-specific is vocabulary
//! and behaviour that cannot be data in the file: the default field
//! catalogue behind profiles, flag names, semantic query phrases, the
//! instruction decoder, and diff-alignment hints. Each console family
//! contributes one [`Family`] here (see `docs/multi-system.md`).

use crate::profile::SubsystemDef;
use crate::query::Condition;

pub mod gb;
pub mod nes;

/// Field-catalogue construction shorthand shared by the family catalogues.
macro_rules! field {
    ($name:expr, u8) => {
        FieldDef { name: $name, field_type: FieldType::UInt8, nullable: false, dictionary: false }
    };
    ($name:expr, u8, dict) => {
        FieldDef { name: $name, field_type: FieldType::UInt8, nullable: false, dictionary: true }
    };
    ($name:expr, u16) => {
        FieldDef { name: $name, field_type: FieldType::UInt16, nullable: false, dictionary: false }
    };
    ($name:expr, u16, nullable) => {
        FieldDef { name: $name, field_type: FieldType::UInt16, nullable: true, dictionary: false }
    };
    ($name:expr, u8, nullable) => {
        FieldDef { name: $name, field_type: FieldType::UInt8, nullable: true, dictionary: false }
    };
    ($name:expr, bool) => {
        FieldDef { name: $name, field_type: FieldType::Bool, nullable: false, dictionary: true }
    };
    ($name:expr, str, nullable) => {
        FieldDef { name: $name, field_type: FieldType::Str, nullable: true, dictionary: false }
    };
}
pub(crate) use field;


/// A named CPU flag: which field holds it and at which bit. The first name
/// is canonical (single letter); the rest are accepted aliases.
pub struct FlagDef {
    pub names: &'static [&'static str],
    pub field: &'static str,
    pub bit: u8,
}

/// A console family: the system-specific vocabulary and behaviour behind
/// profiles, queries, disassembly, and diff alignment.
pub struct Family {
    /// Identifier stored in the trace header and profile (`"gb"`).
    pub id: &'static str,

    /// Default field catalogue: validates profiles and types legacy traces
    /// whose headers predate `field_defs`.
    pub subsystems: &'static [&'static SubsystemDef],

    /// Flag vocabulary for `flag …` queries and viewer flag rendering,
    /// in display order (high bit first).
    pub flags: &'static [FlagDef],

    /// Semantic query phrases that are exactly one fixed string
    /// (`"lcd on"`), desugaring to a generic [`Condition`].
    pub exact_phrases: &'static [(&'static str, fn() -> Condition)],

    /// Semantic query phrases of the form `<prefix><number>`
    /// (`"interrupt 2"`), with an inclusive maximum for the number.
    pub numbered_phrases: &'static [(&'static str, u8, fn(u8) -> Condition)],

    /// Instruction decoder: (rom, address) → (mnemonic, length).
    pub disassemble: Option<fn(&[u8], u16) -> (String, u8)>,

    /// Diff-alignment hint: the address every trace of this system reaches
    /// at program entry, and the address of the entry's second instruction
    /// (proves execution continued past it). GB: cartridge entry
    /// 0x0100/0x0101.
    pub entry_addrs: Option<(u16, u16)>,
}

impl Family {
    /// Look up a field definition by name across this family's subsystems.
    pub fn lookup_field(&self, name: &str) -> Option<&'static crate::profile::FieldDef> {
        self.subsystems
            .iter()
            .flat_map(|s| s.all_fields())
            .find(|f| f.name == name)
    }
}

/// Every registered family. GB first — it is also the fallback for traces
/// whose headers predate the `family` field.
pub static FAMILIES: &[&Family] = &[&gb::GB, &nes::NES];

/// Look up a family by id.
pub fn family(id: &str) -> Option<&'static Family> {
    FAMILIES.iter().copied().find(|f| f.id == id)
}
