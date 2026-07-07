//! TraceComparison: lightweight comparison view over two TraceStores.
//!
//! Instead of copying/transforming traces for comparison, TraceComparison holds
//! references to two original stores and maintains index mappings for
//! alignment (sync) and downsampling (tcycle → instruction collapse).
//!
//! No data is copied. All reads go through the index maps to the originals.

use arrow::array::*;
use arrow::array::types::UInt8Type;

use crate::store::{ColumnSegment, TraceStore};
use crate::error::{Error, Result};
use crate::profile::FieldType;

/// A comparison view over two trace stores.
pub struct TraceComparison<'a> {
    pub store_a: &'a dyn TraceStore,
    pub store_b: &'a dyn TraceStore,
    /// Maps aligned index → original entry index in store A.
    pub map_a: Vec<usize>,
    /// Maps aligned index → original entry index in store B.
    pub map_b: Vec<usize>,
    /// Cached per-field diff stats.
    field_stats: Option<Vec<FieldDiffStats>>,
}

/// Per-field diff statistics.
#[derive(Debug, Clone)]
pub struct FieldDiffStats {
    pub name: String,
    pub match_count: usize,
    pub diff_count: usize,
}

impl FieldDiffStats {
    pub fn match_pct(&self) -> f64 {
        let total = self.match_count + self.diff_count;
        if total == 0 { return 100.0; }
        self.match_count as f64 / total as f64 * 100.0
    }
}

impl<'a> TraceComparison<'a> {
    /// Create a TraceComparison by aligning two traces.
    ///
    /// Alignment works on the instruction address (`op_addr`, falling back to
    /// `pc` for older traces) so T-cycle traces align on instruction
    /// boundaries rather than mid-instruction `pc` values.
    ///
    /// Sync modes:
    /// - `None` or `Some("auto")` — pick the best built-in mode for the inputs.
    ///   If both traces start at instruction address 0x0100 (cartridge ROM
    ///   entry), advance both to the first 0x0101 to skip the post-boot
    ///   WriteOp tail and the first NOP (where adapters' clocks land at
    ///   different sub-phases). Otherwise fall back to first-common-address
    ///   alignment.
    /// - `Some("cartridge")` — explicitly require cartridge-entry alignment;
    ///   errors if either trace's first entry isn't at 0x0100.
    /// - `Some("pc")` — first-common-address alignment (legacy default).
    /// - `Some("none")` — no alignment, compare from entry 0.
    /// - `Some("field=value")` / `Some("field&mask")` — advance both stores to
    ///   the first entry matching the condition. Values are parsed as hex
    ///   (with or without `0x` prefix), matching the `query --where` syntax.
    pub fn align(
        store_a: &'a dyn TraceStore,
        store_b: &'a dyn TraceStore,
        sync: Option<&str>,
    ) -> Result<Self> {
        let a_tcycle = matches!(store_a.header().trigger, crate::header::Trigger::Tcycle);
        let b_tcycle = matches!(store_b.header().trigger, crate::header::Trigger::Tcycle);

        // When triggers differ (mcycle vs tcycle, instruction vs tcycle, etc.) collapse
        // BOTH traces to PC-change boundaries. Otherwise the side with finer granularity
        // has more entries per instruction (e.g. an M-cycle trace emits one entry per
        // M-cycle, including ones where PC doesn't change like LD (HL+),A's write step),
        // which drifts the alignment by one per such instruction.
        let triggers_differ = matches!(
            (&store_a.header().trigger, &store_b.header().trigger),
            (a, b) if a != b
        );
        let collapse_a = (a_tcycle && !b_tcycle) || triggers_differ;
        let collapse_b = (b_tcycle && !a_tcycle) || triggers_differ;

        let mut map_a = if collapse_a {
            collapse_indices(store_a)?
        } else {
            (0..store_a.entry_count()).collect()
        };

        let mut map_b = if collapse_b {
            collapse_indices(store_b)?
        } else {
            (0..store_b.entry_count()).collect()
        };

        // Apply sync alignment
        let sync_mode = sync.unwrap_or("auto");
        match sync_mode {
            "none" => {}
            "auto" => {
                if !try_align_cartridge_entry(store_a, store_b, &mut map_a, &mut map_b) {
                    align_by_pc(store_a, store_b, &mut map_a, &mut map_b);
                }
            }
            "cartridge" => {
                if !try_align_cartridge_entry(store_a, store_b, &mut map_a, &mut map_b) {
                    return Err(Error::Diff(
                        "sync=cartridge: both traces must start at PC=0x0100 \
                         (cartridge ROM entry) and contain a later PC=0x0101 entry"
                            .into(),
                    ));
                }
            }
            "pc" => {
                align_by_pc(store_a, store_b, &mut map_a, &mut map_b);
            }
            condition => {
                align_by_condition(store_a, store_b, &mut map_a, &mut map_b, condition)?;
            }
        }

        // Truncate to the shorter of the two
        let len = map_a.len().min(map_b.len());
        map_a.truncate(len);
        map_b.truncate(len);

        Ok(Self {
            store_a,
            store_b,
            map_a,
            map_b,
            field_stats: None,
        })
    }

    /// Number of aligned entry pairs.
    pub fn len(&self) -> usize {
        self.map_a.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map_a.is_empty()
    }

    /// Get the original entry index in store A for an aligned index.
    pub fn original_a(&self, aligned_idx: usize) -> usize {
        self.map_a[aligned_idx]
    }

    /// Get the original entry index in store B for an aligned index.
    pub fn original_b(&self, aligned_idx: usize) -> usize {
        self.map_b[aligned_idx]
    }

    /// Check if a specific field differs at an aligned index.
    pub fn field_differs(&self, field: &str, aligned_idx: usize) -> bool {
        let col_a = self.store_a.field_col(field);
        let col_b = self.store_b.field_col(field);
        match (col_a, col_b) {
            (Some(ca), Some(cb)) => {
                let row_a = self.map_a[aligned_idx];
                let row_b = self.map_b[aligned_idx];
                let ft = self.store_a.header().resolve_field_type(field);
                match ft {
                    FieldType::Bool => {
                        self.store_a.get_bool(ca, row_a) != self.store_b.get_bool(cb, row_b)
                    }
                    FieldType::Str => {
                        self.store_a.get_str(ca, row_a) != self.store_b.get_str(cb, row_b)
                    }
                    _ => {
                        self.store_a.get_numeric(ca, row_a) != self.store_b.get_numeric(cb, row_b)
                    }
                }
            }
            _ => false, // field not in both stores
        }
    }

    /// Compute per-field diff statistics (cached after first call).
    pub fn compute_stats(&mut self) -> &[FieldDiffStats] {
        self.compute_stats_filtered(None)
    }

    /// Compute per-field diff statistics, optionally restricted to specific fields.
    pub fn compute_stats_filtered(&mut self, filter: Option<&[&str]>) -> &[FieldDiffStats] {
        if let Some(ref stats) = self.field_stats {
            return stats;
        }

        let fields_a = &self.store_a.header().fields;
        let fields_b = &self.store_b.header().fields;
        let len = self.len();

        // Find common fields, applying filter
        let common: Vec<&String> = fields_a.iter()
            .filter(|f| fields_b.contains(f))
            .filter(|f| match filter {
                Some(allowed) => allowed.contains(&f.as_str()),
                None => true,
            })
            .collect();

        // Try bulk column comparison if maps are contiguous
        let bulk_a = is_contiguous(&self.map_a);
        let bulk_b = is_contiguous(&self.map_b);

        let stats = if let (Some((start_a, _)), Some((start_b, _))) = (bulk_a, bulk_b) {
            // Both maps are contiguous: use column-oriented bulk comparison
            bulk_compare_fields(
                self.store_a, self.store_b,
                &common, len, start_a, start_b,
            )
        } else {
            // Non-contiguous maps: per-entry comparison (still filtered)
            scalar_compare_fields(
                self.store_a, self.store_b,
                &common, &self.map_a, &self.map_b,
            )
        };

        self.field_stats = Some(stats);
        self.field_stats.as_ref().unwrap()
    }

    /// Overall match percentage across all common fields.
    pub fn overall_match_pct(&mut self) -> f64 {
        let stats = self.compute_stats();
        let total_matches: usize = stats.iter().map(|s| s.match_count).sum();
        let total_diffs: usize = stats.iter().map(|s| s.diff_count).sum();
        let total = total_matches + total_diffs;
        if total == 0 { return 100.0; }
        total_matches as f64 / total as f64 * 100.0
    }
}

// ---------------------------------------------------------------------------
// Bulk column comparison
// ---------------------------------------------------------------------------

/// Check if an index map is a contiguous range [start..start+len).
fn is_contiguous(map: &[usize]) -> Option<(usize, usize)> {
    if map.is_empty() { return Some((0, 0)); }
    let start = map[0];
    let end = start + map.len();
    if map[map.len() - 1] != end - 1 { return None; }
    // Spot-check middle to avoid false positives
    if map.len() > 2 {
        let mid = map.len() / 2;
        if map[mid] != start + mid { return None; }
    }
    Some((start, end))
}

/// Compare fields using bulk column access (both maps contiguous).
fn bulk_compare_fields(
    store_a: &dyn TraceStore,
    store_b: &dyn TraceStore,
    fields: &[&String],
    len: usize,
    start_a: usize,
    start_b: usize,
) -> Vec<FieldDiffStats> {
    fields.iter().map(|field| {
        let end_a = start_a + len;
        let end_b = start_b + len;

        let segs_a = store_a.get_column_segments(field, start_a, end_a);
        let segs_b = store_b.get_column_segments(field, start_b, end_b);

        let diff_count = match (segs_a, segs_b) {
            (Some(sa), Some(sb)) => compare_segments(&sa, &sb),
            _ => {
                // Fallback to scalar for this field
                scalar_diff_count(store_a, store_b, field, len, start_a, start_b)
            }
        };

        FieldDiffStats {
            name: field.to_string(),
            match_count: len - diff_count,
            diff_count,
        }
    }).collect()
}

/// Count differences between two segment lists by walking them in lockstep.
fn compare_segments(segs_a: &[ColumnSegment], segs_b: &[ColumnSegment]) -> usize {
    let mut diffs = 0;
    let mut iter_a = SegmentIter::new(segs_a);
    let mut iter_b = SegmentIter::new(segs_b);

    while let (Some((arr_a, off_a)), Some((arr_b, off_b))) =
        (iter_a.current(), iter_b.current())
    {

        let avail_a = segment_remaining(&iter_a);
        let avail_b = segment_remaining(&iter_b);
        let batch = avail_a.min(avail_b);

        diffs += count_diffs_typed(arr_a, off_a, arr_b, off_b, batch);

        iter_a.advance(batch);
        iter_b.advance(batch);
    }

    diffs
}

/// Typed comparison of Arrow arrays — avoids per-element dispatch.
fn count_diffs_typed(
    a: &ArrayRef, off_a: usize,
    b: &ArrayRef, off_b: usize,
    len: usize,
) -> usize {
    // Try common numeric types first (most fields are u8/u16)
    if let (Some(aa), Some(bb)) = (
        a.as_any().downcast_ref::<UInt8Array>(),
        b.as_any().downcast_ref::<UInt8Array>(),
    ) {
        let sa = &aa.values()[off_a..off_a + len];
        let sb = &bb.values()[off_b..off_b + len];
        return sa.iter().zip(sb).filter(|(x, y)| x != y).count();
    }

    if let (Some(aa), Some(bb)) = (
        a.as_any().downcast_ref::<UInt16Array>(),
        b.as_any().downcast_ref::<UInt16Array>(),
    ) {
        let sa = &aa.values()[off_a..off_a + len];
        let sb = &bb.values()[off_b..off_b + len];
        return sa.iter().zip(sb).filter(|(x, y)| x != y).count();
    }

    if let (Some(aa), Some(bb)) = (
        a.as_any().downcast_ref::<UInt32Array>(),
        b.as_any().downcast_ref::<UInt32Array>(),
    ) {
        let sa = &aa.values()[off_a..off_a + len];
        let sb = &bb.values()[off_b..off_b + len];
        return sa.iter().zip(sb).filter(|(x, y)| x != y).count();
    }

    if let (Some(aa), Some(bb)) = (
        a.as_any().downcast_ref::<BooleanArray>(),
        b.as_any().downcast_ref::<BooleanArray>(),
    ) {
        return (0..len)
            .filter(|&i| aa.value(off_a + i) != bb.value(off_b + i))
            .count();
    }

    if let (Some(aa), Some(bb)) = (
        a.as_any().downcast_ref::<StringArray>(),
        b.as_any().downcast_ref::<StringArray>(),
    ) {
        return (0..len)
            .filter(|&i| aa.value(off_a + i) != bb.value(off_b + i))
            .count();
    }

    // Dictionary-encoded u8
    if let (Some(da), Some(db)) = (
        a.as_any().downcast_ref::<DictionaryArray<UInt8Type>>(),
        b.as_any().downcast_ref::<DictionaryArray<UInt8Type>>(),
    ) {
        let va = da.values().as_any().downcast_ref::<UInt8Array>().unwrap();
        let vb = db.values().as_any().downcast_ref::<UInt8Array>().unwrap();
        let ka = da.keys();
        let kb = db.keys();
        return (0..len)
            .filter(|&i| {
                va.value(ka.value(off_a + i) as usize) != vb.value(kb.value(off_b + i) as usize)
            })
            .count();
    }

    // Unknown type: treat all as different
    len
}

/// Iterator over segments for lockstep traversal.
struct SegmentIter<'a> {
    segs: &'a [ColumnSegment],
    seg_idx: usize,
    pos_in_seg: usize,
}

impl<'a> SegmentIter<'a> {
    fn new(segs: &'a [ColumnSegment]) -> Self {
        Self { segs, seg_idx: 0, pos_in_seg: 0 }
    }

    fn current(&self) -> Option<(&'a ArrayRef, usize)> {
        let seg = self.segs.get(self.seg_idx)?;
        Some((&seg.array, seg.offset + self.pos_in_seg))
    }

    fn advance(&mut self, n: usize) {
        self.pos_in_seg += n;
        if let Some(seg) = self.segs.get(self.seg_idx) {
            if self.pos_in_seg >= seg.len {
                self.seg_idx += 1;
                self.pos_in_seg = 0;
            }
        }
    }
}

fn segment_remaining(iter: &SegmentIter) -> usize {
    match iter.segs.get(iter.seg_idx) {
        Some(seg) => seg.len - iter.pos_in_seg,
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// Scalar fallback (non-contiguous maps or no bulk access)
// ---------------------------------------------------------------------------

/// Per-entry comparison for non-contiguous alignment maps.
fn scalar_compare_fields(
    store_a: &dyn TraceStore,
    store_b: &dyn TraceStore,
    fields: &[&String],
    map_a: &[usize],
    map_b: &[usize],
) -> Vec<FieldDiffStats> {
    let len = map_a.len();
    fields.iter().map(|field| {
        let diff_count = scalar_diff_count_mapped(store_a, store_b, field, map_a, map_b);
        FieldDiffStats {
            name: field.to_string(),
            match_count: len - diff_count,
            diff_count,
        }
    }).collect()
}

fn scalar_diff_count_mapped(
    store_a: &dyn TraceStore,
    store_b: &dyn TraceStore,
    field: &str,
    map_a: &[usize],
    map_b: &[usize],
) -> usize {
    let col_a = match store_a.field_col(field) { Some(c) => c, None => return 0 };
    let col_b = match store_b.field_col(field) { Some(c) => c, None => return 0 };
    let ft = store_a.header().resolve_field_type(field);

    map_a.iter().zip(map_b).filter(|(&ra, &rb)| {
        match ft {
            FieldType::Bool => store_a.get_bool(col_a, ra) != store_b.get_bool(col_b, rb),
            FieldType::Str => store_a.get_str(col_a, ra) != store_b.get_str(col_b, rb),
            _ => store_a.get_numeric(col_a, ra) != store_b.get_numeric(col_b, rb),
        }
    }).count()
}

/// Scalar diff count for contiguous ranges (fallback when bulk segments unavailable).
fn scalar_diff_count(
    store_a: &dyn TraceStore,
    store_b: &dyn TraceStore,
    field: &str,
    len: usize,
    start_a: usize,
    start_b: usize,
) -> usize {
    let col_a = match store_a.field_col(field) { Some(c) => c, None => return 0 };
    let col_b = match store_b.field_col(field) { Some(c) => c, None => return 0 };
    let ft = store_a.header().resolve_field_type(field);

    (0..len).filter(|&i| {
        let ra = start_a + i;
        let rb = start_b + i;
        match ft {
            FieldType::Bool => store_a.get_bool(col_a, ra) != store_b.get_bool(col_b, rb),
            FieldType::Str => store_a.get_str(col_a, ra) != store_b.get_str(col_b, rb),
            _ => store_a.get_numeric(col_a, ra) != store_b.get_numeric(col_b, rb),
        }
    }).count()
}

// ---------------------------------------------------------------------------
// Standalone bulk comparison (for WASM and other consumers)
// ---------------------------------------------------------------------------

/// Count differences for a single field between two stores over a contiguous range.
/// Uses bulk column access when available, falling back to scalar.
pub fn bulk_field_diff_count(
    store_a: &dyn TraceStore,
    store_b: &dyn TraceStore,
    field: &str,
    start: usize,
    len: usize,
) -> usize {
    let end = start + len;
    let segs_a = store_a.get_column_segments(field, start, end);
    let segs_b = store_b.get_column_segments(field, start, end);

    match (segs_a, segs_b) {
        (Some(sa), Some(sb)) => compare_segments(&sa, &sb),
        _ => scalar_diff_count(store_a, store_b, field, len, start, start),
    }
}

/// Find indices where a field differs between two stores.
/// Uses bulk column access when available.
pub fn bulk_field_diff_indices(
    store_a: &dyn TraceStore,
    store_b: &dyn TraceStore,
    field: &str,
    start: usize,
    len: usize,
) -> Vec<u32> {
    let end = start + len;
    let segs_a = store_a.get_column_segments(field, start, end);
    let segs_b = store_b.get_column_segments(field, start, end);

    match (segs_a, segs_b) {
        (Some(sa), Some(sb)) => diff_indices_from_segments(&sa, &sb, start),
        _ => {
            // Scalar fallback
            let col_a = match store_a.field_col(field) { Some(c) => c, None => return vec![] };
            let col_b = match store_b.field_col(field) { Some(c) => c, None => return vec![] };
            (0..len)
                .filter(|&i| {
                    store_a.get_numeric(col_a, start + i) != store_b.get_numeric(col_b, start + i)
                })
                .map(|i| (start + i) as u32)
                .collect()
        }
    }
}

fn diff_indices_from_segments(segs_a: &[ColumnSegment], segs_b: &[ColumnSegment], global_start: usize) -> Vec<u32> {
    let mut indices = Vec::new();
    let mut iter_a = SegmentIter::new(segs_a);
    let mut iter_b = SegmentIter::new(segs_b);
    let mut global_pos = global_start;

    while let (Some((arr_a, off_a)), Some((arr_b, off_b))) =
        (iter_a.current(), iter_b.current())
    {

        let avail_a = segment_remaining(&iter_a);
        let avail_b = segment_remaining(&iter_b);
        let batch = avail_a.min(avail_b);

        collect_diff_indices_typed(arr_a, off_a, arr_b, off_b, batch, global_pos, &mut indices);

        global_pos += batch;
        iter_a.advance(batch);
        iter_b.advance(batch);
    }

    indices
}

fn collect_diff_indices_typed(
    a: &ArrayRef, off_a: usize,
    b: &ArrayRef, off_b: usize,
    len: usize,
    global_start: usize,
    out: &mut Vec<u32>,
) {
    if let (Some(aa), Some(bb)) = (
        a.as_any().downcast_ref::<UInt8Array>(),
        b.as_any().downcast_ref::<UInt8Array>(),
    ) {
        let sa = &aa.values()[off_a..off_a + len];
        let sb = &bb.values()[off_b..off_b + len];
        for (i, (x, y)) in sa.iter().zip(sb).enumerate() {
            if x != y { out.push((global_start + i) as u32); }
        }
        return;
    }

    // Fallback: use UInt16 comparison (next most common type)
    if let (Some(aa), Some(bb)) = (
        a.as_any().downcast_ref::<UInt16Array>(),
        b.as_any().downcast_ref::<UInt16Array>(),
    ) {
        let sa = &aa.values()[off_a..off_a + len];
        let sb = &bb.values()[off_b..off_b + len];
        for (i, (x, y)) in sa.iter().zip(sb).enumerate() {
            if x != y { out.push((global_start + i) as u32); }
        }
        return;
    }

    // Last resort: per-element via generic comparison
    for i in 0..len {
        out.push((global_start + i) as u32);
    }
}

// ---------------------------------------------------------------------------
// Alignment helpers
// ---------------------------------------------------------------------------

/// Build an index map that collapses T-cycle entries to instruction
/// boundaries. Picks one entry per instruction-address change (`op_addr`,
/// which ticks exactly once per instruction; falling back to `pc`, which also
/// advances on operand reads).
fn collapse_indices(store: &dyn TraceStore) -> Result<Vec<usize>> {
    let addr_col = store.addr_col()
        .ok_or_else(|| Error::Diff("no pc/op_addr field for collapse".into()))?;
    let count = store.entry_count();
    if count == 0 { return Ok(vec![]); }

    let mut indices = vec![0]; // always include first entry
    let mut prev_addr = store.get_numeric(addr_col, 0);

    for i in 1..count {
        let cur_addr = store.get_numeric(addr_col, i);
        if cur_addr != prev_addr {
            indices.push(i);
        }
        prev_addr = cur_addr;
    }

    Ok(indices)
}

/// First entry in `map` whose instruction-address column equals `target`.
/// Returns the position within `map` (not the original row index).
fn find_pc_position(store: &dyn TraceStore, addr_col: usize, map: &[usize], target: u16) -> Option<usize> {
    map.iter().position(|&i| store.get_numeric(addr_col, i) as u16 == target)
}

/// Align index maps by first common instruction address (`op_addr`, falling
/// back to `pc`).
fn align_by_pc(
    store_a: &dyn TraceStore,
    store_b: &dyn TraceStore,
    map_a: &mut Vec<usize>,
    map_b: &mut Vec<usize>,
) {
    let addr_col_a = store_a.addr_col();
    let addr_col_b = store_b.addr_col();

    if let (Some(ca), Some(cb)) = (addr_col_a, addr_col_b) {
        if map_a.is_empty() || map_b.is_empty() { return; }

        let pc_a = store_a.get_numeric(ca, map_a[0]) as u16;
        let pc_b = store_b.get_numeric(cb, map_b[0]) as u16;

        if pc_a == pc_b { return; } // already aligned

        // Look for the other side's first address in our first 100 entries.
        let head_a = &map_a[..map_a.len().min(100)];
        let head_b = &map_b[..map_b.len().min(100)];
        let target = find_pc_position(store_a, ca, head_a, pc_b).map(|_| pc_b)
            .or_else(|| find_pc_position(store_b, cb, head_b, pc_a).map(|_| pc_a));

        if let Some(target_pc) = target {
            if pc_a != target_pc {
                if let Some(pos) = find_pc_position(store_a, ca, map_a, target_pc) {
                    map_a.drain(..pos);
                }
            }
            if pc_b != target_pc {
                if let Some(pos) = find_pc_position(store_b, cb, map_b, target_pc) {
                    map_b.drain(..pos);
                }
            }
        }
    }
}

/// Align index maps by first match of a condition string.
fn align_by_condition(
    store_a: &dyn TraceStore,
    store_b: &dyn TraceStore,
    map_a: &mut Vec<usize>,
    map_b: &mut Vec<usize>,
    condition: &str,
) -> Result<()> {
    let (field, op, value) = if let Some(pos) = condition.find('&') {
        (&condition[..pos], '&', &condition[pos + 1..])
    } else if let Some(pos) = condition.find('=') {
        (&condition[..pos], '=', &condition[pos + 1..])
    } else {
        return Err(Error::Diff(format!("invalid sync condition: {condition}")));
    };

    let val = crate::query::parse_number(value)
        .ok_or_else(|| Error::Diff(format!("invalid value: {value}")))?;

    let matches_condition = |store: &dyn TraceStore, row: usize| -> bool {
        if let Some(col) = store.field_col(field) {
            let v = store.get_numeric(col, row);
            match op {
                '&' => (v & val) != 0,
                '=' => v == val,
                _ => false,
            }
        } else {
            false
        }
    };

    if let Some(pos) = map_a.iter().position(|&idx| matches_condition(store_a, idx)) {
        map_a.drain(..pos);
    }
    if let Some(pos) = map_b.iter().position(|&idx| matches_condition(store_b, idx)) {
        map_b.drain(..pos);
    }

    Ok(())
}

/// Advance both maps to the first entry at the family's second entry-point
/// address, but only if both traces start at the family's entry point
/// (GB: cartridge entry 0x0100, conventionally NOP, with JP nn at 0x0101 —
/// aligning past the entry NOP skips the post-boot WriteOp tail, where
/// adapters land at different M-cycle sub-phases; see `missingno-gb`'s
/// `Cpu::new` comment about the in-flight `LDH (FF50), A` residual).
/// Returns `true` when alignment was applied. Used by `auto` and
/// `cartridge` sync modes.
fn try_align_cartridge_entry(
    store_a: &dyn TraceStore,
    store_b: &dyn TraceStore,
    map_a: &mut Vec<usize>,
    map_b: &mut Vec<usize>,
) -> bool {
    let (entry, after_entry) = match store_a.header().family_def().entry_addrs {
        Some(addrs) => addrs,
        None => return false,
    };
    let pc_col_a = match store_a.addr_col() { Some(c) => c, None => return false };
    let pc_col_b = match store_b.addr_col() { Some(c) => c, None => return false };
    if map_a.is_empty() || map_b.is_empty() { return false; }

    let pc_a0 = store_a.get_numeric(pc_col_a, map_a[0]) as u16;
    let pc_b0 = store_b.get_numeric(pc_col_b, map_b[0]) as u16;
    if pc_a0 != entry || pc_b0 != entry { return false; }

    match (
        find_pc_position(store_a, pc_col_a, map_a, after_entry),
        find_pc_position(store_b, pc_col_b, map_b, after_entry),
    ) {
        (Some(a), Some(b)) => {
            map_a.drain(..a);
            map_b.drain(..b);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{BootRom, PixFormat, TraceHeader, Trigger};

    /// Minimal in-memory TraceStore for alignment tests. Holds only a `pc`
    /// column — alignment paths only read that field.
    struct PcStore {
        header: TraceHeader,
        pcs: Vec<u16>,
    }

    impl PcStore {
        fn new(pcs: Vec<u16>) -> Self {
            Self {
                header: TraceHeader {
                    _header: true,
                    format_version: "0.1.0".into(),
                    emulator: "test".into(),
                    emulator_version: "0".into(),
                    rom_sha256: "0".into(),
                    model: "DMG".into(),
                    boot_rom: BootRom::Skip,
                    profile: "test".into(),
                    fields: vec!["pc".into()],
                    trigger: Trigger::Tcycle,
                    pix_format: PixFormat::default(),
                    extension_fields: std::collections::BTreeMap::new(),
                    notes: String::new(),
                    ..Default::default()
                },
                pcs,
            }
        }
    }

    impl TraceStore for PcStore {
        fn header(&self) -> &TraceHeader { &self.header }
        fn entry_count(&self) -> usize { self.pcs.len() }
        fn field_col(&self, name: &str) -> Option<usize> {
            if name == "pc" { Some(0) } else { None }
        }
        fn frame_boundaries(&self) -> Vec<u32> { vec![0] }
        fn get_str(&self, _col: usize, row: usize) -> String { format!("{:04x}", self.pcs[row]) }
        fn get_numeric(&self, _col: usize, row: usize) -> u64 { self.pcs[row] as u64 }
        fn get_bool(&self, _col: usize, _row: usize) -> bool { false }
        fn is_null(&self, _col: usize, _row: usize) -> bool { false }
    }

    /// Two-column store carrying both `pc` and `op_addr`, for testing that
    /// instruction-address sync prefers `op_addr` over the mid-instruction
    /// `pc`. Column 0 = `pc`, column 1 = `op_addr`.
    struct RegStore {
        header: TraceHeader,
        pcs: Vec<u16>,
        op_addrs: Vec<u16>,
    }

    impl RegStore {
        fn new(pcs: Vec<u16>, op_addrs: Vec<u16>, trigger: Trigger) -> Self {
            let header = TraceHeader {
                _header: true,
                format_version: "0.1.0".into(),
                emulator: "test".into(),
                emulator_version: "0".into(),
                rom_sha256: "0".into(),
                model: "DMG".into(),
                boot_rom: BootRom::Skip,
                profile: "test".into(),
                fields: vec!["pc".into(), "op_addr".into()],
                trigger,
                pix_format: PixFormat::default(),
                extension_fields: std::collections::BTreeMap::new(),
                notes: String::new(),
                ..Default::default()
            };
            Self { header, pcs, op_addrs }
        }
    }

    impl TraceStore for RegStore {
        fn header(&self) -> &TraceHeader { &self.header }
        fn entry_count(&self) -> usize { self.pcs.len() }
        fn field_col(&self, name: &str) -> Option<usize> {
            match name { "pc" => Some(0), "op_addr" => Some(1), _ => None }
        }
        fn frame_boundaries(&self) -> Vec<u32> { vec![0] }
        fn get_str(&self, col: usize, row: usize) -> String {
            format!("{:04x}", self.get_numeric(col, row))
        }
        fn get_numeric(&self, col: usize, row: usize) -> u64 {
            (if col == 0 { self.pcs[row] } else { self.op_addrs[row] }) as u64
        }
        fn get_bool(&self, _col: usize, _row: usize) -> bool { false }
        fn is_null(&self, _col: usize, _row: usize) -> bool { false }
    }

    #[test]
    fn collapse_and_sync_use_op_addr_not_mid_instruction_pc() {
        // A is a T-cycle trace where `pc` advances through operand reads while
        // `op_addr` stays at the instruction's address. Two instructions:
        // 0x0100 (3 T-cycles, pc walks 0100→0102) then 0x0150 (2 T-cycles).
        let a = RegStore::new(
            vec![0x0100, 0x0101, 0x0102, 0x0150, 0x0151],
            vec![0x0100, 0x0100, 0x0100, 0x0150, 0x0150],
            Trigger::Tcycle,
        );
        // B is an instruction-level trace of the same two instructions.
        let b = RegStore::new(
            vec![0x0100, 0x0150],
            vec![0x0100, 0x0150],
            Trigger::Instruction,
        );

        // Differing triggers force A to collapse to instruction boundaries.
        let cmp = TraceComparison::align(&a, &b, Some("none")).unwrap();
        // Collapsing by `op_addr` yields exactly two instructions; collapsing
        // by the mid-instruction `pc` would yield four (one per pc change).
        assert_eq!(cmp.len(), 2);
        // Second aligned row maps to A's row 3 (op_addr 0x0150), not row 1
        // (pc 0x0101) which a pc-based collapse would have picked.
        assert_eq!(cmp.original_a(1), 3);
        assert_eq!(a.op_addrs[cmp.original_a(1)], 0x0150);
    }

    fn cartridge_like() -> PcStore {
        PcStore::new(vec![0x0100, 0x0100, 0x0100, 0x0101, 0x0101, 0x0102, 0x0103, 0x0150])
    }

    #[test]
    fn auto_skips_post_boot_tail_when_both_at_cartridge_entry() {
        let a = PcStore::new(vec![0x0100, 0x0100, 0x0100, 0x0100, 0x0100, 0x0101, 0x0102]);
        let b = PcStore::new(vec![0x0100, 0x0100, 0x0100, 0x0101, 0x0102]);
        let cmp = TraceComparison::align(&a, &b, None).unwrap();
        // Both should land on PC=0x0101 at aligned index 0, then walk forward
        // truncated to the shorter map (b has 2 entries from PC=0x0101 onward).
        assert_eq!(cmp.len(), 2);
        assert_eq!(a.pcs[cmp.original_a(0)], 0x0101);
        assert_eq!(b.pcs[cmp.original_b(0)], 0x0101);
        assert_eq!(a.pcs[cmp.original_a(1)], 0x0102);
        assert_eq!(b.pcs[cmp.original_b(1)], 0x0102);
    }

    #[test]
    fn auto_falls_back_to_pc_when_not_cartridge_entry() {
        // Neither starts at 0x0100 → cartridge mode declines; fall back to
        // first-common-PC. Both already share 0x0150 at index 0, so no shift.
        let a = PcStore::new(vec![0x0150, 0x0151, 0x0152]);
        let b = PcStore::new(vec![0x0150, 0x0151, 0x0152]);
        let cmp = TraceComparison::align(&a, &b, None).unwrap();
        assert_eq!(cmp.len(), 3);
        assert_eq!(cmp.original_a(0), 0);
        assert_eq!(cmp.original_b(0), 0);
    }

    #[test]
    fn cartridge_mode_errors_when_traces_dont_start_at_0x0100() {
        let a = PcStore::new(vec![0x0150, 0x0151]);
        let b = PcStore::new(vec![0x0150, 0x0151]);
        match TraceComparison::align(&a, &b, Some("cartridge")) {
            Err(Error::Diff(msg)) => assert!(msg.contains("cartridge")),
            Err(other) => panic!("expected Diff error, got {other:?}"),
            Ok(_) => panic!("expected error, alignment succeeded"),
        }
    }

    #[test]
    fn pc_mode_keeps_legacy_first_common_pc_behavior() {
        // Both start at 0x0100 → pc mode is a no-op (already common).
        let a = cartridge_like();
        let b = cartridge_like();
        let cmp = TraceComparison::align(&a, &b, Some("pc")).unwrap();
        assert_eq!(cmp.len(), a.pcs.len());
        assert_eq!(cmp.original_a(0), 0);
    }

    #[test]
    fn condition_mode_parses_value_as_hex_with_or_without_prefix() {
        let a = cartridge_like();
        let b = cartridge_like();

        let cmp_prefixed = TraceComparison::align(&a, &b, Some("pc=0x0101")).unwrap();
        let cmp_bare = TraceComparison::align(&a, &b, Some("pc=0101")).unwrap();
        assert_eq!(cmp_prefixed.len(), cmp_bare.len());
        assert_eq!(a.pcs[cmp_prefixed.original_a(0)], 0x0101);
        assert_eq!(a.pcs[cmp_bare.original_a(0)], 0x0101);
    }
}
