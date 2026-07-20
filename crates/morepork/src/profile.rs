use crate::error::{Error, Result};
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

/// Complete metadata for a single trace field.
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: &'static str,
    pub field_type: FieldType,
    pub nullable: bool,
    pub dictionary: bool,
}

// ---------------------------------------------------------------------------
// Subsystem / layer definitions
// ---------------------------------------------------------------------------

/// A capture layer within a subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    Registers,
    Internal,
    Writes,
    Output,
    Timing,
}

impl Layer {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "registers" => Some(Layer::Registers),
            "internal" => Some(Layer::Internal),
            "writes" => Some(Layer::Writes),
            "output" => Some(Layer::Output),
            "timing" => Some(Layer::Timing),
            _ => None,
        }
    }
}

/// A hardware subsystem definition with its available layers.
pub struct SubsystemDef {
    pub name: &'static str,
    pub layers: &'static [(Layer, &'static [FieldDef])],
}

impl SubsystemDef {
    /// Get all fields for the given layers.
    pub(crate) fn fields_for_layers(&self, layers: &[Layer]) -> Vec<&'static FieldDef> {
        self.layers
            .iter()
            .filter(|(l, _)| layers.contains(l))
            .flat_map(|(_, fields)| fields.iter())
            .collect()
    }

    /// Get all fields across all layers.
    pub(crate) fn all_fields(&self) -> Vec<&'static FieldDef> {
        self.layers
            .iter()
            .flat_map(|(_, fields)| fields.iter())
            .collect()
    }

    /// Get the available layer names for this subsystem.
    pub(crate) fn available_layers(&self) -> Vec<Layer> {
        self.layers.iter().map(|(l, _)| *l).collect()
    }
}

// The GB-catalogue field-lookup free helpers were removed once the missingno
// producer began authoring its trace headers from its own state schema: they
// existed only so that producer could type its emitters through morepork.
// Readers resolve types from the self-describing header (`TraceHeader::resolve_*`);
// a non-missingno GB adapter that still selects fields through a profile reaches
// the catalogue via its system registry entry (`System::lookup_field`).

// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------
//
// **Extension fields.** Adapters can surface emulator-internal debug state
// without changing this catalogue by declaring extension fields in the
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

/// A capture profile loaded from a TOML file.
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub description: String,
    /// The system the profile targets ("dmg" when the TOML omits it).
    pub system: String,
    pub trigger: Trigger,
    /// Flattened, ordered list of field names to capture.
    pub fields: Vec<String>,
    /// Memory address reads: maps field name -> address.
    pub memory: BTreeMap<String, u16>,
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
#[derive(Deserialize, Debug)]
#[serde(untagged)]
enum LayerSelection {
    Bool(bool),
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Deserialize, Default)]
struct FieldGroupsToml {
    /// Arbitrary memory reads: name = "hex_address"
    #[serde(default)]
    memory: BTreeMap<String, String>,
    /// Adapter-defined extension fields. TOML form:
    /// `[fields.extensions]`
    /// `missingno = ["pending_vector_resolve", "halt_bug"]`
    /// Each adapter resolves its own list at trace-creation time.
    #[serde(default)]
    extensions: BTreeMap<String, Vec<String>>,
    /// Every other key is a subsystem layer selection, validated against
    /// the profile's family catalogue.
    #[serde(flatten)]
    subsystems: BTreeMap<String, LayerSelection>,
}

fn parse_hex_addr(s: &str) -> std::result::Result<u16, String> {
    let s = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u16::from_str_radix(s, 16).map_err(|_| format!("invalid hex address: {s}"))
}

fn resolve_layers(
    selection: &LayerSelection,
    subsystem: &SubsystemDef,
) -> std::result::Result<Vec<Layer>, String> {
    match selection {
        LayerSelection::Bool(true) => Ok(subsystem.available_layers()),
        LayerSelection::Bool(false) => Ok(vec![]),
        LayerSelection::Single(s) if s == "all" => Ok(subsystem.available_layers()),
        LayerSelection::Single(s) => {
            let layer = Layer::from_str(s)
                .ok_or_else(|| format!("unknown layer '{s}' for subsystem '{}'", subsystem.name))?;
            if !subsystem.available_layers().contains(&layer) {
                return Err(format!(
                    "subsystem '{}' does not have layer '{s}'",
                    subsystem.name
                ));
            }
            Ok(vec![layer])
        }
        LayerSelection::Multiple(layers) => {
            let mut result = Vec::new();
            for s in layers {
                if s == "all" {
                    return Ok(subsystem.available_layers());
                }
                let layer = Layer::from_str(s).ok_or_else(|| {
                    format!("unknown layer '{s}' for subsystem '{}'", subsystem.name)
                })?;
                if !subsystem.available_layers().contains(&layer) {
                    return Err(format!(
                        "subsystem '{}' does not have layer '{s}'",
                        subsystem.name
                    ));
                }
                if !result.contains(&layer) {
                    result.push(layer);
                }
            }
            Ok(result)
        }
    }
}

impl Profile {
    /// Load a profile from a TOML file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Self::parse(&contents)
    }

    /// Parse a profile from a TOML string.
    pub fn parse(toml_str: &str) -> Result<Self> {
        let raw: ProfileToml = toml::from_str(toml_str)?;

        let system_id = raw.profile.system.as_deref().unwrap_or("dmg");
        let system = crate::system::system(system_id).ok_or_else(|| {
            let known: Vec<&str> = crate::system::SYSTEMS.iter().map(|s| s.id).collect();
            Error::Profile(format!(
                "unknown system '{system_id}': expected one of {}",
                known.join(", ")
            ))
        })?;

        // Reject subsystem keys the system doesn't have (previously a typo'd
        // key was silently ignored).
        for key in raw.fields.subsystems.keys() {
            if !system.subsystems.iter().any(|s| s.name == key) {
                let known: Vec<&str> = system.subsystems.iter().map(|s| s.name).collect();
                return Err(Error::Profile(format!(
                    "unknown subsystem '{key}' for system '{}': expected one of {}",
                    system.id,
                    known.join(", ")
                )));
            }
        }

        // Resolve each subsystem's layer selection into fields, in the
        // family's catalogue order (not TOML key order).
        let mut fields = Vec::new();
        for subsystem in system.subsystems {
            if let Some(sel) = raw.fields.subsystems.get(subsystem.name) {
                let layers = resolve_layers(sel, subsystem).map_err(Error::Profile)?;
                for field_def in subsystem.fields_for_layers(&layers) {
                    if fields.contains(&field_def.name.to_string()) {
                        return Err(Error::Profile(format!(
                            "duplicate field: {}",
                            field_def.name
                        )));
                    }
                    fields.push(field_def.name.to_string());
                }
            }
        }

        // Parse memory address fields
        let mut memory = BTreeMap::new();
        for (name, addr_str) in &raw.fields.memory {
            if fields.contains(name) || system.lookup_field(name).is_some() {
                return Err(Error::Profile(format!(
                    "memory field '{name}' conflicts with a built-in field"
                )));
            }
            let addr = parse_hex_addr(addr_str)
                .map_err(|e| Error::Profile(format!("memory field '{name}': {e}")))?;
            fields.push(name.clone());
            memory.insert(name.clone(), addr);
        }

        // Extensions don't add anything to `fields` here — adapters merge
        // their own extension list into `fields` at trace-creation time
        // (when they know which adapter they are). Validate names don't
        // shadow built-ins or memory entries.
        for (adapter, ext_fields) in &raw.fields.extensions {
            for name in ext_fields {
                if system.lookup_field(name).is_some() {
                    return Err(Error::Profile(format!(
                        "extensions.{adapter}: '{name}' shadows a built-in field"
                    )));
                }
                if memory.contains_key(name) {
                    return Err(Error::Profile(format!(
                        "extensions.{adapter}: '{name}' conflicts with a memory field"
                    )));
                }
            }
        }

        Ok(Profile {
            name: raw.profile.name,
            description: raw.profile.description,
            system: system.id.to_string(),
            trigger: raw.profile.trigger,
            fields,
            memory,
            extensions: raw.fields.extensions,
        })
    }
}
