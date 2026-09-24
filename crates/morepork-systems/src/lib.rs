//! The system registry: each system's column vocabulary, obtained from
//! missingno's state schemas.
//!
//! A column a producer may write is one of three things: a field of the
//! system's schema, one of the trace observations every corpus-driven
//! producer carries (`missingno_trace::TRACE_OBSERVATIONS`), or one of the
//! system's own capture-bridge observations (the Game Boy's `op_addr`, the
//! VCS's `line`). Names, types, subsystems and layers all come from missingno;
//! this crate only gathers them per system, types a producer's columns
//! against them, and expands profile selections over them.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use missingno_core::state::SystemStateSchema;
use missingno_trace::{build_columns, ObservationDef, TraceObservation, TraceScope};
use morepork::header::{HeaderFieldDef, TraceHeader};
use morepork::profile::Profile;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown system '{0}': expected one of {known}", known = SYSTEM_IDS.join(", "))]
    UnknownSystem(String),

    #[error("system '{system}' has no column '{column}'")]
    UnknownColumn { system: String, column: String },

    #[error("extension field '{column}' shadows a column of system '{system}'")]
    ShadowedColumn { system: String, column: String },

    #[error("profile error: {0}")]
    Profile(String),

    #[error(transparent)]
    Morepork(#[from] morepork::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Every system id the registry knows.
pub static SYSTEM_IDS: &[&str] = &["dmg", "cgb", "vcs", "sg1000", "colecovision", "msx1"];

/// The MSX1 carries the SG-1000's CPU and VDP behind its own slot machinery,
/// and missingno has no MSX1 system; its vocabulary is those two chips' own.
static MSX1_SCHEMA: LazyLock<SystemStateSchema> = LazyLock::new(|| {
    let mut fields = missingno_zilog_z80::record::state_fields();
    fields.extend(missingno_ti_vdp::record::state_fields());
    fields.sort_by_key(|field| field.tier);
    SystemStateSchema {
        system: "msx1",
        isa: "z80",
        instruction_addr_field: "pc",
        entry: None,
        fields,
        memory: Vec::new(),
        frame: missingno_sg1000::state_schema::sg1000_state_schema().frame,
    }
});

/// A system's state schema.
pub fn schema(id: &str) -> Option<&'static SystemStateSchema> {
    Some(match id {
        "dmg" => missingno_gb::state_schema::dmg_state_schema(),
        "cgb" => missingno_gbc::state_schema::cgb_state_schema(),
        "vcs" => missingno_vcs::state_schema::vcs_state_schema(),
        "sg1000" => missingno_sg1000::state_schema::sg1000_state_schema(),
        "colecovision" => missingno_colecovision::state_schema::colecovision_state_schema(),
        "msx1" => &MSX1_SCHEMA,
        _ => return None,
    })
}

/// The trace observations every corpus-driven producer carries.
pub fn trace_observations() -> &'static [ObservationDef<TraceObservation>] {
    missingno_trace::TRACE_OBSERVATIONS
}

/// The observations a system's missingno capture bridge adds beside its
/// schema, as header field defs.
pub fn bridge_observations(id: &str) -> Result<Vec<HeaderFieldDef>> {
    let schema = schema(id).ok_or_else(|| Error::UnknownSystem(id.to_string()))?;
    Ok(match id {
        "dmg" | "cgb" => observation_defs(schema, missingno_gb::trace::OBSERVATIONS),
        "vcs" => observation_defs(schema, missingno_vcs::trace::OBSERVATIONS),
        _ => Vec::new(),
    })
}

/// The header defs of an observation table alone.
fn observation_defs<T: Copy>(
    schema: &SystemStateSchema,
    observations: &'static [ObservationDef<T>],
) -> Vec<HeaderFieldDef> {
    let (_, mut defs) = build_columns(schema, TraceScope::Full, observations);
    defs.split_off(defs.len() - observations.len())
}

/// Every column a system's traces may carry: the schema's fields (observable,
/// then boundary), then the bridge's observations, then the trace
/// observations.
fn vocabulary(id: &str) -> Result<&'static [HeaderFieldDef]> {
    static VOCABULARIES: LazyLock<BTreeMap<&'static str, Vec<HeaderFieldDef>>> =
        LazyLock::new(|| {
            SYSTEM_IDS
                .iter()
                .map(|&id| {
                    let schema = schema(id).expect("every registered id has a schema");
                    let (_, mut defs) =
                        build_columns(schema, TraceScope::Full, trace_observations());
                    let trace = defs.split_off(defs.len() - trace_observations().len());
                    let bridge = bridge_observations(id).expect("registered id");
                    for def in bridge.into_iter().chain(trace) {
                        if !defs.iter().any(|d| d.name == def.name) {
                            defs.push(def);
                        }
                    }
                    (id, defs)
                })
                .collect()
        });
    VOCABULARIES
        .get(id)
        .map(Vec::as_slice)
        .ok_or_else(|| Error::UnknownSystem(id.to_string()))
}

/// Type a producer's columns for a system. A name outside the system's
/// vocabulary is an error naming the system and the column.
pub fn column_defs(id: &str, names: &[&str]) -> Result<Vec<HeaderFieldDef>> {
    let vocabulary = vocabulary(id)?;
    names
        .iter()
        .map(|name| {
            vocabulary
                .iter()
                .find(|d| d.name == *name)
                .cloned()
                .ok_or_else(|| Error::UnknownColumn {
                    system: id.to_string(),
                    column: name.to_string(),
                })
        })
        .collect()
}

/// Complete a producer's header from the registry: the ISA and entry hint
/// its system states, the system's instruction-address column when the
/// producer carries it, and — when the producer gave none — a def for every
/// column (its declared extension fields as declared, the rest through
/// [`column_defs`]). An extension field may not shadow a vocabulary name.
pub fn describe(header: &mut TraceHeader) -> Result<()> {
    let schema =
        schema(&header.system).ok_or_else(|| Error::UnknownSystem(header.system.clone()))?;
    if header.isa.is_empty() {
        header.isa = schema.isa.to_string();
    }
    if header.entry_addrs.is_none() {
        header.entry_addrs = schema.entry;
    }
    if header.instruction_addr_field.is_none()
        && header
            .fields
            .iter()
            .any(|f| f == schema.instruction_addr_field)
    {
        header.instruction_addr_field = Some(schema.instruction_addr_field.to_string());
    }
    let vocabulary = vocabulary(&header.system)?;
    if let Some(name) = header
        .extension_fields
        .keys()
        .find(|name| vocabulary.iter().any(|d| &d.name == *name))
    {
        return Err(Error::ShadowedColumn {
            system: header.system.clone(),
            column: name.clone(),
        });
    }
    if header.field_defs.is_empty() {
        let mut defs = Vec::with_capacity(header.fields.len());
        for name in &header.fields {
            defs.push(match header.extension_fields.get(name) {
                Some(ext) => ext.field_def(name),
                None => column_defs(&header.system, &[name.as_str()])?.remove(0),
            });
        }
        header.field_defs = defs;
    }
    Ok(())
}

/// Load a profile and expand its selections over its system's vocabulary.
pub fn load_profile(path: impl AsRef<std::path::Path>) -> Result<Profile> {
    let mut profile = Profile::load(path)?;
    expand_profile(&mut profile)?;
    Ok(profile)
}

/// Parse a profile and expand its selections over its system's vocabulary.
pub fn parse_profile(toml: &str) -> Result<Profile> {
    let mut profile = Profile::parse(toml)?;
    expand_profile(&mut profile)?;
    Ok(profile)
}

/// Expand a profile's subsystem-layer selections (`cpu = "registers"`) into
/// field names — each selected subsystem's columns of the selected layers, in
/// vocabulary order. A layer is a schema tier's name (`registers` for
/// observable, `internal` for boundary) or an observation's own layer.
pub fn expand_profile(profile: &mut Profile) -> Result<()> {
    let system = profile.system.clone();
    let vocabulary = vocabulary(&system)?;

    let mut subsystems: Vec<&str> = Vec::new();
    for def in vocabulary {
        if let Some(s) = def.subsystem.as_deref() {
            if !subsystems.contains(&s) {
                subsystems.push(s);
            }
        }
    }
    for key in profile.selections.keys() {
        if !subsystems.contains(&key.as_str()) {
            return Err(Error::Profile(format!(
                "unknown subsystem '{key}' for system '{system}': expected one of {}",
                subsystems.join(", ")
            )));
        }
    }

    let mut fields = Vec::new();
    for &subsystem in &subsystems {
        let Some(selection) = profile.selections.get(subsystem) else {
            continue;
        };
        let in_subsystem = vocabulary
            .iter()
            .filter(|d| d.subsystem.as_deref() == Some(subsystem))
            .collect::<Vec<_>>();
        let mut available: Vec<&str> = Vec::new();
        for def in &in_subsystem {
            if let Some(layer) = def.layer.as_deref() {
                if !available.contains(&layer) {
                    available.push(layer);
                }
            }
        }
        let layers = selection
            .resolve(subsystem, &available)
            .map_err(Error::Profile)?;
        for layer in layers {
            for def in in_subsystem
                .iter()
                .filter(|d| d.layer.as_deref() == Some(layer))
            {
                fields.push(def.name.clone());
            }
        }
    }

    let in_vocabulary = |name: &str| vocabulary.iter().any(|d| d.name == name);
    for (adapter, names) in &profile.extensions {
        for name in names {
            if in_vocabulary(name) {
                return Err(Error::Profile(format!(
                    "extensions.{adapter}: '{name}' shadows a built-in field"
                )));
            }
        }
    }

    profile.fields = fields;
    Ok(())
}
