//! Query conditions for filtering and searching trace entries.
//!
//! Conditions range from simple field comparisons to stateful transition
//! detection. System-semantic phrases ("lcd on", "flag z set") desugar to
//! the generic conditions through vocabulary tables; the `Condition` enum
//! itself is system-agnostic.

/// A condition that can be evaluated against trace entries.
///
/// Some conditions are stateless (e.g. `FieldEquals`) and can be checked
/// against a single entry. Others are stateful (e.g. `FieldChanges`) and
/// compare against the previous entry. Evaluation lives with the trace
/// store (`TraceStore::eval_condition_trait`), which reads both rows by
/// column.
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

// ---------------------------------------------------------------------------
// Condition parsing from strings, against a family's vocabulary
// ---------------------------------------------------------------------------
//
// Flag names and semantic phrases desugar to the generic conditions above;
// only the family's tables (`family::Family`) know register names and bit
// meanings.

/// Parse a number from condition syntax. Always treats the digits as hex
/// (with or without a `0x` prefix).
pub fn parse_number(s: &str) -> Option<u64> {
    let s = s.trim();
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u64::from_str_radix(hex, 16).ok()
}

/// Map a CPU flag name to the family's flag definition.
fn flag_def<'f>(
    family: &'f crate::family::Family,
    name: &str,
) -> Result<&'f crate::family::FlagDef, String> {
    let name = name.to_lowercase();
    family
        .flags
        .iter()
        .find(|d| d.names.contains(&name.as_str()))
        .ok_or_else(|| {
            let expected: Vec<&str> = family.flags.iter().map(|d| d.names[0]).collect();
            format!("unknown flag '{name}': expected {}", expected.join(", "))
        })
}

/// Parse a condition from a human-readable string.
///
/// Generic formats, valid for every family:
/// - `field=value` — field equals value
/// - `field changes` / `field changes to value` / `field changes from value`
/// - `field & mask` / `field & mask = value` — bitwise tests
/// - `flag <name> set/clear/becomes set/becomes clear` — through the
///   family's flag vocabulary
///
/// Plus the family's semantic phrases (`exact_phrases`,
/// `numbered_phrases`), e.g. the GB's `lcd on` or `ppu enters mode N`.
pub fn parse_condition(
    s: &str,
    family: &crate::family::Family,
) -> Result<Condition, String> {
    let s = s.trim();

    // Flag conditions: "flag z set", "flag c clear", "flag z becomes set", etc.
    if let Some(rest) = s.strip_prefix("flag ") {
        let rest = rest.trim();
        // "flag z becomes set" / "flag z becomes clear"
        if let Some(inner) = rest.strip_suffix(" becomes set") {
            let d = flag_def(family, inner.trim())?;
            return Ok(Condition::BitTransition { field: d.field.into(), bit: d.bit, to: true });
        }
        if let Some(inner) = rest.strip_suffix(" becomes clear") {
            let d = flag_def(family, inner.trim())?;
            return Ok(Condition::BitTransition { field: d.field.into(), bit: d.bit, to: false });
        }
        // "flag z set" / "flag z clear"
        if let Some(inner) = rest.strip_suffix(" set") {
            let d = flag_def(family, inner.trim())?;
            return Ok(Condition::FieldBitMask { field: d.field.into(), mask: 1 << d.bit });
        }
        if let Some(inner) = rest.strip_suffix(" clear") {
            let d = flag_def(family, inner.trim())?;
            return Ok(Condition::FieldBitMaskEquals {
                field: d.field.into(),
                mask: 1 << d.bit,
                value: 0,
            });
        }
        return Err(format!("invalid flag condition: '{s}'. Expected: flag z set, flag c clear, flag z becomes set, flag c becomes clear"));
    }

    // Semantic phrases
    for (phrase, build) in family.exact_phrases {
        if s == *phrase {
            return Ok(build());
        }
    }
    for (prefix, max, build) in family.numbered_phrases {
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
