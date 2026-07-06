//! Query conditions for filtering and searching trace entries.
//!
//! Conditions range from simple field comparisons to stateful transition
//! detection. System-semantic phrases ("lcd on", "flag z set") desugar to
//! the generic conditions through vocabulary tables; the `Condition` enum
//! itself is system-agnostic.

use crate::entry::TraceEntry;
use serde_json::Value;

/// A condition that can be evaluated against trace entries.
///
/// Some conditions are stateless (e.g. `FieldEquals`) and can be checked
/// against a single entry. Others are stateful (e.g. `FieldChanges`) and
/// require tracking the previous entry — use [`ConditionEvaluator`] for those.
#[derive(Debug, Clone)]
pub enum Condition {
    // --- Stateless: single-entry checks ---
    /// Field equals a specific value (string comparison on the display form).
    FieldEquals { field: String, value: String },

    // --- Stateful: require previous entry ---
    /// Field changed to any different value since the previous entry.
    FieldChanges { field: String },

    /// Field changed to a specific value (was something else before).
    FieldChangesTo { field: String, value: String },

    /// Field changed from a specific value (was that value, now isn't).
    FieldChangesFrom { field: String, value: String },

    /// A single bit transitions to the given state (from its complement).
    /// The generic form behind "becomes set"/"becomes clear" queries and
    /// semantic phrases like `lcd on` and `interrupt N`.
    BitTransition { field: String, bit: u8, to: bool },

    /// A masked view of a field transitions to a specific value: matches
    /// when `cur & mask == value` and the previous entry's masked value
    /// differed (or there is no previous entry). Behind `ppu enters mode N`.
    MaskedChangesTo { field: String, mask: u64, value: u64 },

    /// A counter field wraps: its value decreases sharply from near the top
    /// of its range (heuristic: `cur < prev && prev > 0x80`). Behind
    /// `timer overflow`.
    FieldWraps { field: String },

    /// Bitwise-AND test: `(field & mask) != 0`. Generalises per-bit
    /// queries; e.g., `if_ & 0x02` matches whenever the STAT IRQ bit is
    /// set, irrespective of other IF bits.
    FieldBitMask { field: String, mask: u64 },

    /// Bitwise-AND equality test: `(field & mask) == value`. E.g.,
    /// `stat & 0x03 = 1` matches the VBlank PPU mode robustly to other
    /// STAT bits.
    FieldBitMaskEquals { field: String, mask: u64, value: u64 },

    // --- Compound ---
    /// All sub-conditions must match.
    All(Vec<Condition>),

    /// Any sub-condition must match.
    Any(Vec<Condition>),
}

impl Condition {
    /// Whether this condition requires state from the previous entry.
    pub fn is_stateful(&self) -> bool {
        match self {
            Condition::FieldEquals { .. }
            | Condition::FieldBitMask { .. }
            | Condition::FieldBitMaskEquals { .. } => false,
            Condition::FieldChanges { .. }
            | Condition::FieldChangesTo { .. }
            | Condition::FieldChangesFrom { .. }
            | Condition::BitTransition { .. }
            | Condition::MaskedChangesTo { .. }
            | Condition::FieldWraps { .. } => true,
            Condition::All(cs) | Condition::Any(cs) => cs.iter().any(|c| c.is_stateful()),
        }
    }
}

/// Evaluates conditions against a stream of trace entries, tracking
/// state for transition-based conditions.
pub struct ConditionEvaluator {
    condition: Condition,
    prev: Option<TraceEntry>,
}

impl ConditionEvaluator {
    pub fn new(condition: Condition) -> Self {
        Self {
            condition,
            prev: None,
        }
    }

    /// Check whether the current entry matches the condition,
    /// given the tracked previous entry state.
    /// Call this for each entry in order.
    pub fn evaluate(&mut self, entry: &TraceEntry) -> bool {
        let result = eval_condition(&self.condition, entry, self.prev.as_ref());
        self.prev = Some(entry.clone());
        result
    }

    /// Reset the evaluator state (e.g. when starting a new trace).
    pub fn reset(&mut self) {
        self.prev = None;
    }
}

// ---------------------------------------------------------------------------
// Internal evaluation
// ---------------------------------------------------------------------------

fn eval_condition(cond: &Condition, entry: &TraceEntry, prev: Option<&TraceEntry>) -> bool {
    match cond {
        Condition::FieldEquals { field, value } => {
            match entry.get(field) {
                Some(Value::Number(n)) => {
                    // Compare numerically: parse the condition value as hex or decimal
                    if let Some(target) = parse_number(value) {
                        n.as_u64() == Some(target)
                    } else {
                        false
                    }
                }
                Some(v) => entry_field_str_raw(v) == *value,
                None => false,
            }
        }

        Condition::FieldChanges { field } => {
            let cur = entry_field_str(entry, field);
            let prv = prev.and_then(|p| entry_field_str(p, field));
            cur.is_some() && cur != prv
        }

        Condition::FieldChangesTo { field, value } => {
            let matches_val = |e: &TraceEntry| field_matches_value(e, field, value);
            matches_val(entry) && prev.map_or(true, |p| !matches_val(p))
        }

        Condition::FieldChangesFrom { field, value } => {
            let matches_val = |e: &TraceEntry| field_matches_value(e, field, value);
            prev.map_or(false, |p| matches_val(p)) && !matches_val(entry)
        }

        Condition::BitTransition { field, bit, to } => {
            bit_transitions(entry, prev, field, *bit, !*to, *to)
        }

        Condition::MaskedChangesTo { field, mask, value } => {
            let masked = |e: &TraceEntry| e.get(field).and_then(|v| v.as_u64()).map(|n| n & mask);
            masked(entry) == Some(*value)
                && prev.and_then(masked) != Some(*value)
        }

        Condition::FieldWraps { field } => {
            // Heuristic: a sharp decrease from near the top of the range
            // (e.g. TIMA wrapping to its TMA reload value).
            let cur = entry.get(field).and_then(|v| v.as_u64());
            let prv = prev.and_then(|p| p.get(field)).and_then(|v| v.as_u64());
            match (cur, prv) {
                (Some(c), Some(p)) => c < p && p > 0x80,
                _ => false,
            }
        }

        Condition::FieldBitMask { field, mask } => {
            entry.get(field).and_then(|v| v.as_u64()).map_or(false, |n| (n & mask) != 0)
        }

        Condition::FieldBitMaskEquals { field, mask, value } => {
            entry.get(field).and_then(|v| v.as_u64()).map_or(false, |n| (n & mask) == *value)
        }

        Condition::All(cs) => cs.iter().all(|c| eval_condition(c, entry, prev)),
        Condition::Any(cs) => cs.iter().any(|c| eval_condition(c, entry, prev)),
    }
}

/// Get a field value as its raw string representation.
fn entry_field_str(entry: &TraceEntry, field: &str) -> Option<String> {
    entry.get(field).map(|v| entry_field_str_raw(v))
}

fn entry_field_str_raw(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        _ => v.to_string(),
    }
}

/// Parse a number from user input. Always treats as hex (with or without 0x prefix).
pub fn parse_number(s: &str) -> Option<u64> {
    let s = s.trim();
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u64::from_str_radix(hex, 16).ok()
}

/// Check if a field in an entry matches a value string (numeric or string comparison).
fn field_matches_value(entry: &TraceEntry, field: &str, value: &str) -> bool {
    match entry.get(field) {
        Some(Value::Number(n)) => {
            if let Some(target) = parse_number(value) {
                n.as_u64() == Some(target)
            } else {
                false
            }
        }
        Some(v) => entry_field_str_raw(v) == *value,
        None => false,
    }
}

/// Check if a specific bit in a numeric field transitioned between two states.
fn bit_transitions(
    entry: &TraceEntry,
    prev: Option<&TraceEntry>,
    field: &str,
    bit: u8,
    from: bool,
    to: bool,
) -> bool {
    let cur_val = entry.get(field).and_then(|v| v.as_u64());
    let prv_val = prev.and_then(|p| p.get(field)).and_then(|v| v.as_u64());
    match (cur_val, prv_val) {
        (Some(c), Some(p)) => {
            let cur_bit = (c >> bit) & 1 == 1;
            let prv_bit = (p >> bit) & 1 == 1;
            prv_bit == from && cur_bit == to
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Condition parsing from strings
// ---------------------------------------------------------------------------

/// Parse a condition from a human-readable string.
///
/// Supported formats:
/// - `field=value` — field equals value
/// - `field changes` — field changes to any value
/// - `field changes to value` — field transitions to specific value
/// - `field changes from value` — field transitions from specific value
/// - `ppu enters mode N` — PPU enters mode 0-3
/// - `lcd on` / `lcd off` — LCD turns on/off
/// - `timer overflow` — TIMA overflows
/// - `interrupt N` — interrupt bit N fires (0=vblank, 1=stat, 2=timer, 3=serial, 4=joypad)
// ---------------------------------------------------------------------------
// System-semantic vocabulary
// ---------------------------------------------------------------------------
//
// Flag names and semantic phrases desugar to the generic conditions above;
// only these tables know register names and bit meanings. Game Boy content
// for now — they become per-family tables when the family registry lands
// (docs/multi-system.md).

/// A named CPU flag: which field holds it and at which bit. The first name
/// is canonical (single letter); the rest are accepted aliases.
pub struct FlagDef {
    pub names: &'static [&'static str],
    pub field: &'static str,
    pub bit: u8,
}

static FLAGS: &[FlagDef] = &[
    FlagDef { names: &["z", "zero"], field: "f", bit: 7 },
    FlagDef { names: &["n", "sub", "subtract"], field: "f", bit: 6 },
    FlagDef { names: &["h", "half", "halfcarry"], field: "f", bit: 5 },
    FlagDef { names: &["c", "carry"], field: "f", bit: 4 },
];

/// The flag vocabulary, in display order (high bit first). Consumers (the
/// web viewer's flag rendering and query chips) read it from here rather
/// than hard-coding register names.
pub fn flag_defs() -> &'static [FlagDef] {
    FLAGS
}

/// A semantic phrase that is exactly one fixed string.
static EXACT_PHRASES: &[(&str, fn() -> Condition)] = &[
    ("lcd on", || Condition::BitTransition { field: "lcdc".into(), bit: 7, to: true }),
    ("lcd off", || Condition::BitTransition { field: "lcdc".into(), bit: 7, to: false }),
    ("timer overflow", || Condition::FieldWraps { field: "tima".into() }),
];

/// A semantic phrase of the form `<prefix><number>`, with an inclusive
/// maximum for the numeric argument.
static NUMBERED_PHRASES: &[(&str, u8, fn(u8) -> Condition)] = &[
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

/// Map a CPU flag name to the field/bit holding it.
fn flag_def(name: &str) -> Result<&'static FlagDef, String> {
    let name = name.to_lowercase();
    FLAGS
        .iter()
        .find(|d| d.names.contains(&name.as_str()))
        .ok_or_else(|| {
            let expected: Vec<&str> = FLAGS.iter().map(|d| d.names[0]).collect();
            format!("unknown flag '{name}': expected {}", expected.join(", "))
        })
}

pub fn parse_condition(s: &str) -> Result<Condition, String> {
    let s = s.trim();

    // Flag conditions: "flag z set", "flag c clear", "flag z becomes set", etc.
    if let Some(rest) = s.strip_prefix("flag ") {
        let rest = rest.trim();
        // "flag z becomes set" / "flag z becomes clear"
        if let Some(inner) = rest.strip_suffix(" becomes set") {
            let d = flag_def(inner.trim())?;
            return Ok(Condition::BitTransition { field: d.field.into(), bit: d.bit, to: true });
        }
        if let Some(inner) = rest.strip_suffix(" becomes clear") {
            let d = flag_def(inner.trim())?;
            return Ok(Condition::BitTransition { field: d.field.into(), bit: d.bit, to: false });
        }
        // "flag z set" / "flag z clear"
        if let Some(inner) = rest.strip_suffix(" set") {
            let d = flag_def(inner.trim())?;
            return Ok(Condition::FieldBitMask { field: d.field.into(), mask: 1 << d.bit });
        }
        if let Some(inner) = rest.strip_suffix(" clear") {
            let d = flag_def(inner.trim())?;
            return Ok(Condition::FieldBitMaskEquals {
                field: d.field.into(),
                mask: 1 << d.bit,
                value: 0,
            });
        }
        return Err(format!("invalid flag condition: '{s}'. Expected: flag z set, flag c clear, flag z becomes set, flag c becomes clear"));
    }

    // Semantic phrases
    for (phrase, build) in EXACT_PHRASES {
        if s == *phrase {
            return Ok(build());
        }
    }
    for (prefix, max, build) in NUMBERED_PHRASES {
        if let Some(rest) = s.strip_prefix(prefix) {
            let n: u8 = rest.trim().parse()
                .map_err(|_| format!("invalid number in '{s}': {rest}"))?;
            if n > *max {
                return Err(format!("'{}' takes 0-{max}, got {n}", prefix.trim_end()));
            }
            return Ok(build(n));
        }
    }

    // "field changes to value"
    if let Some(rest) = s.strip_suffix(" changes") {
        return Ok(Condition::FieldChanges { field: rest.trim().to_string() });
    }
    if s.contains(" changes to ") {
        let parts: Vec<&str> = s.splitn(2, " changes to ").collect();
        return Ok(Condition::FieldChangesTo {
            field: parts[0].trim().to_string(),
            value: parts[1].trim().to_string(),
        });
    }
    if s.contains(" changes from ") {
        let parts: Vec<&str> = s.splitn(2, " changes from ").collect();
        return Ok(Condition::FieldChangesFrom {
            field: parts[0].trim().to_string(),
            value: parts[1].trim().to_string(),
        });
    }

    // Bitwise-AND forms: must be checked BEFORE plain `=` so that
    // `field & mask = value` is parsed as FieldBitMaskEquals, not as
    // FieldEquals on the literal `field & mask` field name.
    if let Some(amp) = s.find('&') {
        let field = s[..amp].trim().to_string();
        let rest = s[amp + 1..].trim();
        if field.is_empty() {
            return Err(format!("invalid bitmask condition '{s}': field must be non-empty"));
        }
        // `field & mask = value`
        if let Some(eq) = rest.find('=') {
            let mask_str = rest[..eq].trim();
            let value_str = rest[eq + 1..].trim();
            let mask = parse_number(mask_str)
                .ok_or_else(|| format!("invalid mask in '{s}': '{mask_str}' is not a number"))?;
            let value = parse_number(value_str)
                .ok_or_else(|| format!("invalid value in '{s}': '{value_str}' is not a number"))?;
            return Ok(Condition::FieldBitMaskEquals { field, mask, value });
        }
        // `field & mask`
        let mask = parse_number(rest)
            .ok_or_else(|| format!("invalid mask in '{s}': '{rest}' is not a number"))?;
        return Ok(Condition::FieldBitMask { field, mask });
    }

    // field=value
    if let Some(eq) = s.find('=') {
        let field = s[..eq].trim().to_string();
        let value = s[eq + 1..].trim().to_string();
        if field.is_empty() || value.is_empty() {
            return Err(format!("invalid condition '{s}': field and value must be non-empty"));
        }
        return Ok(Condition::FieldEquals { field, value });
    }

    Err(format!("cannot parse condition: '{s}'"))
}
