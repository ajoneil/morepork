use crate::error::Result;
use crate::header::Trigger;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Field metadata
// ---------------------------------------------------------------------------

/// Native type of a trace field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    #[serde(rename = "u64")]
    UInt64,
    #[serde(rename = "u16")]
    UInt16,
    #[serde(rename = "u8")]
    UInt8,
    #[serde(rename = "bool")]
    Bool,
    #[serde(rename = "str")]
    Str,
}

// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------
//
// **Extension fields.** Adapters can surface emulator-internal debug state
// outside the system's state vocabulary by declaring extension fields in the
// trace header. A profile opts into them via:
//
// ```toml
// [fields.extensions]
// missingno = ["pending_vector_resolve", "halt_bug"]
// gateboy   = ["intf_latch", "halt_latch"]
// ```
//
// Each adapter consumes its own entry at trace-creation time and ignores
// others. The adapter is responsible for resolving each name to a
// `header::ExtensionField` (declaring `field_type`, nullable, optional
// description / source) and appending the name to `header.fields`.
// Readers consult `TraceHeader::resolve_field_type` for typing — no need
// for any consumer to recompile to handle new extensions.

/// A capture profile loaded from a TOML file. Parsing reads the TOML; the
/// subsystem-layer selections name the system's state vocabulary, so they
/// are expanded into `fields` by `morepork-systems`, which holds it.
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub description: String,
    /// The system the profile targets ("dmg" when the TOML omits it).
    pub system: String,
    pub trigger: Trigger,
    /// Subsystem → layer selection, as the TOML states it.
    pub selections: BTreeMap<String, LayerSelection>,
    /// Flattened, ordered list of field names to capture: the expanded
    /// selections. Empty until the selections are expanded.
    pub fields: Vec<String>,
    /// Adapter-defined extension fields. Maps adapter name (e.g.
    /// "missingno", "gateboy") to a list of extension field names that
    /// adapter should emit. The Profile carries names only; type/metadata
    /// resolution happens in the adapter's own extension registry at
    /// trace-creation time. Adapters silently skip entries keyed on
    /// other adapters' names.
    pub extensions: BTreeMap<String, Vec<String>>,
}

// ---------------------------------------------------------------------------
// TOML deserialization
// ---------------------------------------------------------------------------

/// Raw TOML structure for deserialization.
#[derive(Deserialize)]
struct ProfileToml {
    profile: ProfileMeta,
    fields: FieldGroupsToml,
}

#[derive(Deserialize)]
struct ProfileMeta {
    name: String,
    description: String,
    trigger: Trigger,
    /// The system this profile targets. Absent ⇒ "dmg".
    #[serde(default)]
    system: Option<String>,
}

/// Subsystem layer selection in TOML.
///
/// Each subsystem can be:
/// - `true` or `"all"` — all layers
/// - `"registers"` — a single layer
/// - `["registers", "internal"]` — multiple layers
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum LayerSelection {
    Bool(bool),
    Single(String),
    Multiple(Vec<String>),
}

impl LayerSelection {
    /// The selected layer names out of those the subsystem has, in the
    /// subsystem's order.
    pub fn resolve<'a>(
        &self,
        subsystem: &str,
        available: &[&'a str],
    ) -> std::result::Result<Vec<&'a str>, String> {
        let named: Vec<&str> = match self {
            LayerSelection::Bool(true) => return Ok(available.to_vec()),
            LayerSelection::Bool(false) => return Ok(vec![]),
            LayerSelection::Single(s) => vec![s.as_str()],
            LayerSelection::Multiple(layers) => layers.iter().map(String::as_str).collect(),
        };
        if named.contains(&"all") {
            return Ok(available.to_vec());
        }
        for name in &named {
            if !available.contains(name) {
                return Err(format!(
                    "subsystem '{subsystem}' does not have layer '{name}': expected one of {}",
                    available.join(", ")
                ));
            }
        }
        Ok(available.iter().copied().filter(|l| named.contains(l)).collect())
    }
}

#[derive(Deserialize, Default)]
struct FieldGroupsToml {
    /// Adapter-defined extension fields. TOML form:
    /// `[fields.extensions]`
    /// `missingno = ["pending_vector_resolve", "halt_bug"]`
    /// Each adapter resolves its own list at trace-creation time.
    #[serde(default)]
    extensions: BTreeMap<String, Vec<String>>,
    /// Every other key is a subsystem layer selection.
    #[serde(flatten)]
    subsystems: BTreeMap<String, LayerSelection>,
}

impl Profile {
    /// Load a profile from a TOML file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Self::parse(&contents)
    }

    /// Parse a profile from a TOML string, leaving its selections unexpanded.
    pub fn parse(toml_str: &str) -> Result<Self> {
        let raw: ProfileToml = toml::from_str(toml_str)?;

        Ok(Profile {
            name: raw.profile.name,
            description: raw.profile.description,
            system: raw.profile.system.unwrap_or_else(|| "dmg".to_string()),
            trigger: raw.profile.trigger,
            selections: raw.fields.subsystems,
            fields: Vec::new(),
            extensions: raw.fields.extensions,
        })
    }
}
