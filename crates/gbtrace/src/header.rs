use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::format::FieldGroup;
use crate::profile::FieldType;

/// How the boot ROM was handled for this trace.
///
/// Serializes as a plain string:
/// - `"skip"` — no boot ROM, post-boot state was set manually
/// - `"builtin"` — emulator's built-in boot ROM was used
/// - `"stripped:<original>"` — boot entries were removed post-capture
/// - `"<sha256>"` — a specific boot ROM was used, identified by hash
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Default)]
pub enum BootRom {
    /// Boot ROM was skipped; initial state is post-boot.
    Skip,
    /// Emulator's built-in boot ROM was used.
    #[default]
    Builtin,
    /// Boot ROM was used but entries were stripped post-capture.
    /// Contains the original boot_rom value (e.g. the SHA-256 hash).
    Stripped(String),
    /// A specific boot ROM was used, identified by SHA-256.
    Sha256(String),
}

impl BootRom {
    /// Return the stripped variant, preserving the original boot ROM info.
    pub fn to_stripped(&self) -> Self {
        match self {
            BootRom::Skip => BootRom::Skip, // already no boot data
            BootRom::Builtin => BootRom::Stripped("builtin".to_string()),
            BootRom::Stripped(_) => self.clone(), // already stripped
            BootRom::Sha256(hash) => BootRom::Stripped(hash.clone()),
        }
    }
}


impl Serialize for BootRom {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            BootRom::Skip => serializer.serialize_str("skip"),
            BootRom::Builtin => serializer.serialize_str("builtin"),
            BootRom::Stripped(original) => {
                serializer.serialize_str(&format!("stripped:{original}"))
            }
            BootRom::Sha256(hash) => serializer.serialize_str(hash),
        }
    }
}

impl<'de> Deserialize<'de> for BootRom {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "skip" => BootRom::Skip,
            "builtin" => BootRom::Builtin,
            _ if s.starts_with("stripped:") => {
                BootRom::Stripped(s[9..].to_string())
            }
            _ => BootRom::Sha256(s),
        })
    }
}

/// When trace entries are emitted. `Instruction`, `Scanline`, and `Frame`
/// are family-universal; `Mcycle`/`Tcycle` are the GB clock vocabulary and
/// `Cycle` is the plain CPU-cycle cadence of the 6502 families.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Trigger {
    #[default]
    Instruction,
    Mcycle,
    Tcycle,
    Cycle,
    Scanline,
    Frame,
    Custom,
}

/// How pixel data is encoded. DMG output is greyscale, so a 2-bit shade
/// index per pixel suffices; CGB output is colour, so each pixel is a
/// 15-bit RGB555 value written as 4 hex chars. Absent ⇒ `shade2`
/// (back-compat). `Indexed8` is the family-agnostic form: one palette
/// index per pixel, with dimensions and the palette carried in each
/// `frame` snapshot payload (`snapshot::IndexedFrame`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PixFormat {
    #[default]
    Shade2,
    Rgb555,
    Indexed8,
}

/// Adapter-defined field with its type metadata, declared in the trace
/// header. Used for non-standard fields (emulator-internal debug state)
/// that aren't part of the built-in field catalogue. Readers consult
/// `TraceHeader::extension_fields` to resolve types for these names.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtensionField {
    /// Native type of the field's value.
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// Whether the column is nullable.
    #[serde(default, skip_serializing_if = "is_false")]
    pub nullable: bool,
    /// Human-readable description (what the field captures).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Emulator that defined the field (e.g. "missingno"). Allows
    /// downstream tooling to attribute extension semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn is_false(b: &bool) -> bool { !b }

/// Typed declaration of one trace field, carried in the header so the file
/// is self-describing: readers resolve type, nullability, encoding, and
/// semantic grouping from here without consulting the built-in catalogue.
/// Traces written before this existed omit it; readers then fall back to
/// the static catalogue, which therefore remains as the legacy-trace path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeaderFieldDef {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// Hardware subsystem ("cpu", "ppu", …). None for profile-defined
    /// memory watches and extension fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
    /// Capture layer within the subsystem ("registers", "internal", …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub nullable: bool,
    /// Whether the column uses dictionary encoding.
    #[serde(default, skip_serializing_if = "is_false")]
    pub dictionary: bool,
    /// For extension fields: the emulator that defined it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// The header line of a `.gbtrace` file.
///
/// In the JSONL interchange format only `_header: true` is required; every
/// other field has a serde default.
/// Field types are resolved from name via the built-in catalogue, so an emulator
/// can emit data lines with no header and the reader will synthesise one.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TraceHeader {
    /// Always `true`. Identifies this line as the header.
    pub _header: bool,

    /// Spec version (semver).
    #[serde(default = "default_format_version")]
    pub format_version: String,

    /// Emulator identifier (lowercase, no spaces).
    #[serde(default = "default_emulator")]
    pub emulator: String,

    /// Emulator version string.
    #[serde(default)]
    pub emulator_version: String,

    /// SHA-256 hex digest of the ROM file.
    #[serde(default)]
    pub rom_sha256: String,

    /// Console family this trace belongs to ("gb"). Absent (legacy traces)
    /// ⇒ "gb". Distinct from `model`, which names the variant *within* the
    /// family.
    #[serde(default = "default_family")]
    pub family: String,

    /// Hardware model identifier (e.g. "DMG-B", "NTSC").
    #[serde(default)]
    pub model: String,

    /// How the boot ROM was handled.
    #[serde(default)]
    pub boot_rom: BootRom,

    /// Name of the capture profile used.
    #[serde(default)]
    pub profile: String,

    /// Ordered list of field names present in each state entry.
    /// When empty (and JSONL input has no `fields` in its header), the
    /// reader infers field names from the first data line's keys.
    #[serde(default)]
    pub fields: Vec<String>,

    /// When entries are emitted.
    #[serde(default)]
    pub trigger: Trigger,

    /// How the `pix` field encodes pixels (2-bit shade for DMG, RGB555 for CGB).
    #[serde(default)]
    pub pix_format: PixFormat,

    /// Adapter-defined extension fields. Maps field name → type metadata
    /// for fields that aren't in the built-in catalogue. Adapters declare
    /// these at trace-creation time so the writer / reader can construct
    /// appropriate column buffers and a downstream consumer can resolve
    /// types without having to know about adapter-internal fields.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extension_fields: BTreeMap<String, ExtensionField>,

    /// Typed declarations for every name in `fields`, making the trace
    /// self-describing (see [`HeaderFieldDef`]). The writer populates this;
    /// it is empty on traces written before it existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_defs: Vec<HeaderFieldDef>,

    /// The storage grouping used for this file's chunks (each group is one
    /// Arrow IPC block). When empty (legacy traces), readers re-derive the
    /// writer's grouping convention from field names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_groups: Vec<FieldGroup>,

    /// Name of the column holding the current instruction's address, used
    /// for sync, collapse, and disassembly. Absent ⇒ `op_addr`, then `pc`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_addr_field: Option<String>,

    /// Kind name for each numeric snapshot tag, indexed by tag value
    /// (`snapshot_kinds[0]` names tag 0). `frame` and `memory` are
    /// format-level kinds; system-specific state uses the family's
    /// namespaced names (`gb.cpu`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snapshot_kinds: Vec<String>,

    /// Optional freeform notes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
}

fn default_format_version() -> String { "1.0".to_string() }
fn default_emulator() -> String { "unknown".to_string() }
fn default_family() -> String { "gb".to_string() }

impl TraceHeader {
    /// Validate header invariants. Empty `fields` is permitted at this
    /// stage — JSONL inputs may infer them from the first data line.
    pub fn validate(&self) -> crate::error::Result<()> {
        if !self._header {
            return Err(crate::error::Error::InvalidHeader(
                "_header must be true".into(),
            ));
        }
        // Reject extension fields that shadow built-in field names —
        // type resolution would be ambiguous (the built-in catalogue and
        // the header would each claim a type, with no clear winner).
        for name in self.extension_fields.keys() {
            if self.family_def().lookup_field(name).is_some() {
                return Err(crate::error::Error::InvalidHeader(format!(
                    "extension field '{name}' shadows a built-in field"
                )));
            }
        }
        Ok(())
    }

    /// This header's declaration for a field, when self-describing.
    pub fn field_def(&self, name: &str) -> Option<&HeaderFieldDef> {
        self.field_defs.iter().find(|d| d.name == name)
    }

    /// The family this trace belongs to. Unknown or empty ids resolve to
    /// GB — every trace written before the `family` field existed is a GB
    /// trace, and an unknown id still gets working generic tooling.
    pub fn family_def(&self) -> &'static crate::family::Family {
        crate::family::family(&self.family).unwrap_or(&crate::family::gb::GB)
    }

    /// Resolve a field's type from `field_defs` (headers are
    /// self-describing; `ensure_self_describing` fills them from the
    /// family catalogue and `extension_fields` at write time). Unknown
    /// names fall back to `UInt8`.
    pub fn resolve_field_type(&self, name: &str) -> FieldType {
        self.field_def(name)
            .map(|d| d.field_type)
            .unwrap_or(FieldType::UInt8)
    }

    /// Resolve a field's nullability from `field_defs`.
    pub fn resolve_field_nullable(&self, name: &str) -> bool {
        self.field_def(name).map(|d| d.nullable).unwrap_or(false)
    }

    /// Resolve whether a field's column uses dictionary encoding.
    pub fn resolve_field_dictionary(&self, name: &str) -> bool {
        self.field_def(name).map(|d| d.dictionary).unwrap_or(false)
    }

    /// Fill `field_defs` and `instruction_addr_field` from the built-in
    /// catalogue and `extension_fields` when absent. The binary writer
    /// calls this, so every new trace is self-describing regardless of
    /// which producer (FFI adapter, missingno, `convert`) built the header.
    pub fn ensure_self_describing(&mut self) {
        if self.family.is_empty() {
            // Struct-literal construction with `..Default::default()`
            // yields "" (the derive ignores serde defaults).
            self.family = default_family();
        }
        if self.field_defs.is_empty() {
            let family = self.family_def();
            self.field_defs = self
                .fields
                .iter()
                .map(|name| {
                    if let Some(def) = family.lookup_field(name) {
                        let (subsystem, layer) = family
                            .field_group(name)
                            .map(|(s, l)| (Some(s.to_string()), Some(l.to_string())))
                            .unwrap_or((None, None));
                        HeaderFieldDef {
                            name: name.clone(),
                            field_type: def.field_type,
                            subsystem,
                            layer,
                            nullable: def.nullable,
                            dictionary: def.dictionary,
                            source: None,
                        }
                    } else if let Some(ext) = self.extension_fields.get(name) {
                        HeaderFieldDef {
                            name: name.clone(),
                            field_type: ext.field_type,
                            subsystem: None,
                            layer: None,
                            nullable: ext.nullable,
                            dictionary: false,
                            source: ext.source.clone(),
                        }
                    } else {
                        // Profile-defined memory watches and unknown names:
                        // the same u8 fallback readers have always used.
                        HeaderFieldDef {
                            name: name.clone(),
                            field_type: FieldType::UInt8,
                            subsystem: None,
                            layer: None,
                            nullable: false,
                            dictionary: false,
                            source: None,
                        }
                    }
                })
                .collect();
        }
        if self.instruction_addr_field.is_none() {
            self.instruction_addr_field = ["op_addr", "pc"]
                .iter()
                .find(|n| self.fields.iter().any(|f| f == *n))
                .map(|n| n.to_string());
        }
        if self.snapshot_kinds.is_empty() {
            self.snapshot_kinds = ["frame", "memory"]
                .iter()
                .chain(self.family_def().snapshot_kinds)
                .map(|k| k.to_string())
                .collect();
        }
    }

    /// The kind name for a snapshot tag, from the header's `snapshot_kinds`.
    pub fn snapshot_kind_name(&self, tag: u8) -> Option<&str> {
        self.snapshot_kinds.get(tag as usize).map(String::as_str)
    }
}
