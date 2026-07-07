/** Field-display metadata, installed per loaded trace via setFieldMeta().
 *  Headers are self-describing, so every trace installs its own. */
let sixteenBitFields = new Set();
let flagFields = new Map();

/** Install per-trace field metadata from the wasm store.
 *  fieldDefs: store.fieldDefs() — [{name, type, ...}].
 *  flagDefs: store.flagDefs() — [{name, field, bit}] in display order. */
export function setFieldMeta(fieldDefs, flagDefs) {
  sixteenBitFields = new Set(
    (fieldDefs || []).filter((d) => d.type === 'u16').map((d) => d.name));
  flagFields = new Map();
  for (const { name, field, bit } of flagDefs || []) {
    if (!flagFields.has(field)) flagFields.set(field, []);
    flagFields.get(field).push({ name: name.toUpperCase(), bit });
  }
}

/** Whether a field renders as a flags register. */
export function isFlagField(fieldName) {
  return flagFields.has(fieldName);
}

/** Flag chips for the query builder: [{name, flag}] in display order,
 *  where `flag` is the name the query grammar accepts (`flag z set`). */
export function flagChips() {
  const chips = [];
  for (const flags of flagFields.values()) {
    for (const { name } of flags) {
      chips.push({ name, flag: name.toLowerCase() });
    }
  }
  return chips;
}

/** Format a flags register: hex value + flag letters. */
function formatFlags(v, fieldName) {
  const hex = v.toString(16).padStart(2, '0');
  const letters = flagFields.get(fieldName)
    .map(({ name, bit }) => ((v >> bit) & 1 ? name : '·'))
    .join('');
  return `${hex} ${letters}`;
}

/** Format a flags register with per-flag diff highlighting (returns HTML).
 *  diffColor is applied to flags that differ from otherVal. */
export function displayFlagsDiff(v, otherVal, diffColor, fieldName = 'f') {
  if (typeof v !== 'number') return displayVal(v, fieldName);
  const hex = v.toString(16).padStart(2, '0');
  const xor = (typeof otherVal === 'number') ? (v ^ otherVal) : 0;
  const parts = flagFields.get(fieldName).map(({ name, bit }) => {
    const letter = ((v >> bit) & 1) ? name : '·';
    if ((xor >> bit) & 1) {
      return `<span style="color:${diffColor};font-weight:600">${letter}</span>`;
    }
    return letter;
  });
  const hexDiffers = v !== otherVal;
  const hexHtml = hexDiffers ? `<span style="color:${diffColor}">${hex}</span>` : hex;
  return `${hexHtml} ${parts.join('')}`;
}

/** Format a value as zero-padded lowercase hex for display.
 *  If fieldName is provided, uses field-aware width (e.g. pc always 4 digits). */
export function displayVal(v, fieldName) {
  if (v === undefined || v === null) return '';
  if (typeof v === 'number') {
    if (fieldName && flagFields.has(fieldName)) return formatFlags(v, fieldName);
    if (fieldName && sixteenBitFields.has(fieldName)) {
      return v.toString(16).padStart(4, '0');
    }
    if (v <= 0xFF) return v.toString(16).padStart(2, '0');
    if (v <= 0xFFFF) return v.toString(16).padStart(4, '0');
    return v.toString(16);
  }
  const s = String(v);
  if (s.startsWith('0x') || s.startsWith('0X')) return s.slice(2).toLowerCase();
  return s;
}

/** Normalize user hex input for querying.
 *  Strips optional 0x prefix, returns bare lowercase hex string.
 *  The Rust query parser treats all values as hex. */
export function normalizeInput(v) {
  const s = v.trim();
  if (!s) return s;
  const bare = (s.startsWith('0x') || s.startsWith('0X')) ? s.slice(2) : s;
  return bare.toLowerCase();
}
