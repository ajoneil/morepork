//! Writer for the native `.morepork` binary format.
//!
//! Usage:
//! ```ignore
//! let mut w = MoreporkWriter::create("out.morepork", &header, &groups)?;
//! // For each entry:
//! w.set_u8(col, val);
//! w.set_u16(col, val);
//! w.set_null(col);
//! w.finish_entry()?;
//! // At vblank:
//! w.mark_frame(frame_payload)?;  // Option<&[u8]>, family-defined encoding
//! // When done:
//! w.finish()?;
//! ```

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Seek, Write};
use std::sync::Arc;

use arrow::array::*;
use arrow::array::types::UInt8Type;
use arrow::datatypes::*;
use arrow::ipc::writer::StreamWriter;
use arrow::record_batch::RecordBatch;

use crate::error::{Error, Result};
use crate::header::TraceHeader;
use crate::profile::FieldType;

use super::*;

/// Column buffer for accumulating entries before flushing a chunk.
enum ColBuf {
    U8(UInt8Builder),
    U16(UInt16Builder),
    U64(UInt64Builder),
    Bool(BooleanBuilder),
    Str(StringBuilder),
    DictU8(PrimitiveDictionaryBuilder<UInt8Type, UInt8Type>),
}

impl ColBuf {
    fn new(ft: FieldType, dict: bool, capacity: usize) -> Self {
        if dict && matches!(ft, FieldType::UInt8) {
            return Self::DictU8(PrimitiveDictionaryBuilder::new());
        }
        match ft {
            FieldType::UInt64 => Self::U64(UInt64Builder::with_capacity(capacity)),
            FieldType::UInt16 => Self::U16(UInt16Builder::with_capacity(capacity)),
            FieldType::UInt8 => Self::U8(UInt8Builder::with_capacity(capacity)),
            FieldType::Bool => Self::Bool(BooleanBuilder::with_capacity(capacity)),
            FieldType::Str => Self::Str(StringBuilder::with_capacity(capacity, capacity * 2)),
        }
    }

    /// A type-mismatched append records the column's default so columns
    /// stay aligned — silently dropping the value would desynchronise the
    /// chunk's columns and corrupt the file. The debug assertion surfaces
    /// the producer bug in tests.
    fn append_mismatched(&mut self) {
        debug_assert!(false, "trace column setter called with the wrong type");
        match self {
            Self::U8(b) => b.append_value(0),
            Self::U16(b) => b.append_value(0),
            Self::U64(b) => b.append_value(0),
            Self::Bool(b) => b.append_value(false),
            Self::Str(b) => b.append_value(""),
            Self::DictU8(b) => { b.append_value(0); }
        }
    }

    fn append_u8(&mut self, val: u8) {
        match self {
            Self::U8(b) => b.append_value(val),
            Self::DictU8(b) => { b.append_value(val); }
            _ => self.append_mismatched(),
        }
    }

    fn append_u16(&mut self, val: u16) {
        match self {
            Self::U16(b) => b.append_value(val),
            _ => self.append_mismatched(),
        }
    }

    fn append_u64(&mut self, val: u64) {
        match self {
            Self::U64(b) => b.append_value(val),
            _ => self.append_mismatched(),
        }
    }

    fn append_bool(&mut self, val: bool) {
        match self {
            Self::Bool(b) => b.append_value(val),
            _ => self.append_mismatched(),
        }
    }

    fn append_str(&mut self, val: &str) {
        match self {
            Self::Str(b) => b.append_value(val),
            _ => self.append_mismatched(),
        }
    }

    fn append_null(&mut self) {
        match self {
            Self::U8(b) => b.append_null(),
            Self::U16(b) => b.append_null(),
            Self::U64(b) => b.append_null(),
            Self::Bool(b) => b.append_null(),
            Self::Str(b) => b.append_null(),
            Self::DictU8(b) => b.append_null(),
        }
    }

    fn finish(&mut self) -> ArrayRef {
        match self {
            Self::U8(b) => Arc::new(b.finish()),
            Self::U16(b) => Arc::new(b.finish()),
            Self::U64(b) => Arc::new(b.finish()),
            Self::Bool(b) => Arc::new(b.finish()),
            Self::Str(b) => Arc::new(b.finish()),
            Self::DictU8(b) => Arc::new(b.finish()),
        }
    }

}

/// Mapping from field group to its column indices.
struct GroupMapping {
    /// group_index → [col_indices]
    col_indices: Vec<Vec<usize>>,
}

impl GroupMapping {
    fn from_groups(groups: &[FieldGroup], all_fields: &[String]) -> Self {
        let field_to_col: HashMap<&str, usize> = all_fields.iter()
            .enumerate()
            .map(|(i, f)| (f.as_str(), i))
            .collect();

        let mut col_indices = Vec::new();

        for group in groups.iter() {
            let mut cols = Vec::new();
            for field_name in &group.fields {
                if let Some(&ci) = field_to_col.get(field_name.as_str()) {
                    cols.push(ci);
                }
            }
            col_indices.push(cols);
        }

        Self {
            col_indices,
        }
    }
}

/// Writer for the `.morepork` binary format.
pub struct MoreporkWriter {
    out: BufWriter<File>,
    header: TraceHeader,
    group_mapping: GroupMapping,
    columns: Vec<ColBuf>,
    chunk_size: usize,
    entries_in_chunk: usize,
    total_entries: u64,

    // Footer data accumulated during writing
    chunk_index: Vec<ChunkIndexEntry>,
    snapshot_index: Vec<SnapshotIndexEntry>,
}

/// Group fields for chunk storage by their header defs: subsystem name for
/// the registers layer, `<subsystem>_<layer>` otherwise, `other` for fields
/// without a subsystem (memory watches, extensions).
fn groups_from_defs(header: &TraceHeader) -> Vec<FieldGroup> {
    let mut groups: Vec<FieldGroup> = Vec::new();
    for name in &header.fields {
        let group_name = match header.field_def(name) {
            Some(def) => match (&def.subsystem, &def.layer) {
                (Some(s), Some(l)) if l == "registers" => s.clone(),
                (Some(s), Some(l)) => format!("{s}_{l}"),
                _ => "other".to_string(),
            },
            None => "other".to_string(),
        };
        match groups.iter_mut().find(|g| g.name == group_name) {
            Some(g) => g.fields.push(name.clone()),
            None => groups.push(FieldGroup { name: group_name, fields: vec![name.clone()] }),
        }
    }
    groups
}

impl MoreporkWriter {
    /// Create a new writer.
    pub fn create(
        path: impl AsRef<std::path::Path>,
        header: &TraceHeader,
        groups: &[FieldGroup],
    ) -> Result<Self> {
        let file = File::create(path.as_ref())?;
        let mut out = BufWriter::new(file);

        // Write magic + version
        out.write_all(MAGIC)?;
        out.write_all(&[VERSION])?;

        // Every written trace is self-describing: complete field defs, the
        // storage grouping actually used, and the instruction-address
        // column all go into the header.
        let mut header = header.clone();
        header.ensure_self_describing();
        if header.field_groups.is_empty() {
            header.field_groups = if groups.is_empty() {
                // No grouping given: group by the field defs' subsystem and
                // layer. Any grouping is valid — readers follow whatever the
                // header records — so new producers can just pass `&[]`.
                groups_from_defs(&header)
            } else {
                groups.to_vec()
            };
        }

        // Write header (JSON, zstd-compressed)
        let header_json = serde_json::to_string(&header)?;
        let header_compressed = zstd::encode_all(header_json.as_bytes(), 3)
            .map_err(|e| Error::Io(io::Error::other(e)))?;
        out.write_all(&(header_compressed.len() as u32).to_le_bytes())?;
        out.write_all(&header_compressed)?;

        let group_mapping = GroupMapping::from_groups(&header.field_groups, &header.fields);

        let columns: Vec<ColBuf> = header.fields.iter()
            .map(|name| {
                let ft = header.resolve_field_type(name);
                let dict = header.resolve_field_dictionary(name);
                ColBuf::new(ft, dict, DEFAULT_CHUNK_SIZE)
            })
            .collect();

        Ok(Self {
            out,
            header,
            group_mapping,
            columns,
            chunk_size: DEFAULT_CHUNK_SIZE,
            entries_in_chunk: 0,
            total_entries: 0,
            chunk_index: Vec::new(),
            snapshot_index: Vec::new(),
        })
    }

    // --- Column setters (same API as the FFI writer) ---

    pub fn set_u8(&mut self, col: usize, val: u8) { self.columns[col].append_u8(val); }
    pub fn set_u16(&mut self, col: usize, val: u16) { self.columns[col].append_u16(val); }
    pub fn set_u64(&mut self, col: usize, val: u64) { self.columns[col].append_u64(val); }
    pub fn set_bool(&mut self, col: usize, val: bool) { self.columns[col].append_bool(val); }
    pub fn set_str(&mut self, col: usize, val: &str) { self.columns[col].append_str(val); }
    pub fn set_null(&mut self, col: usize) { self.columns[col].append_null(); }

    /// Finish the current entry. Flushes a chunk when full.
    pub fn finish_entry(&mut self) -> Result<()> {
        self.entries_in_chunk += 1;
        self.total_entries += 1;

        if self.entries_in_chunk >= self.chunk_size {
            self.flush_chunk()?;
        }
        Ok(())
    }

    /// Mark a frame boundary at the current entry position, optionally
    /// attaching the frame's pixel payload (raw GB pixels in the header's
    /// pix_format, or a serialized `snapshot::IndexedFrame`).
    pub fn mark_frame(&mut self, framebuffer: Option<&[u8]>) -> Result<()> {
        let payload = framebuffer.unwrap_or(&[]);
        self.write_snapshot(super::TAG_FRAME, payload)
    }

    /// Write a typed snapshot record at the current entry position.
    /// The payload is compressed with zstd and written inline. `tag` is a
    /// format-level tag (`TAG_FRAME`, `TAG_MEMORY`) or one a family claims from
    /// `FAMILY_TAG_BASE` up; the header's `snapshot_kinds` names it.
    pub fn write_snapshot(&mut self, tag: u8, payload: &[u8]) -> Result<()> {
        let compressed = if payload.is_empty() {
            Vec::new()
        } else {
            zstd::encode_all(payload, 3)
                .map_err(|e| Error::Io(io::Error::other(e)))?
        };

        let offset = self.out.stream_position()?;

        // Write snapshot record: tag + type + entry_index + payload_len + payload
        self.out.write_all(SNAPSHOT_TAG)?;
        self.out.write_all(&[tag])?;
        self.out.write_all(&self.total_entries.to_le_bytes())?;
        self.out.write_all(&(compressed.len() as u32).to_le_bytes())?;
        if !compressed.is_empty() {
            self.out.write_all(&compressed)?;
        }

        self.snapshot_index.push(SnapshotIndexEntry {
            snapshot_type: tag,
            offset,
            entry_index: self.total_entries,
            payload_size: compressed.len() as u32,
        });

        Ok(())
    }

    /// Find a field's column index by name.
    pub fn find_field(&self, name: &str) -> Option<usize> {
        self.header.fields.iter().position(|f| f == name)
    }

    /// Flush the current chunk to disk.
    fn flush_chunk(&mut self) -> Result<()> {
        if self.entries_in_chunk == 0 { return Ok(()); }

        let chunk_offset = self.out.stream_position()?;

        // Build Arrow arrays per group, serialize each group with IPC + zstd
        let mut group_blobs: Vec<(u8, Vec<u8>)> = Vec::new(); // (group_id, compressed_data)

        for (gi, group_cols) in self.group_mapping.col_indices.iter().enumerate() {
            if group_cols.is_empty() { continue; }

            // Build schema + arrays for this group
            let mut fields = Vec::new();
            let mut arrays: Vec<ArrayRef> = Vec::new();

            for &ci in group_cols {
                let name = &self.header.fields[ci];
                let array = self.columns[ci].finish();
                let nullable = self.header.resolve_field_nullable(name);
                let field = Field::new(name, array.data_type().clone(), nullable);
                fields.push(field);
                arrays.push(array);
            }

            let schema = Arc::new(Schema::new(fields));
            let batch = RecordBatch::try_new(schema.clone(), arrays)
                .map_err(Error::Arrow)?;

            // Serialize to Arrow IPC stream
            let mut ipc_buf = Vec::new();
            {
                let mut writer = StreamWriter::try_new(&mut ipc_buf, &schema)
                    .map_err(Error::Arrow)?;
                writer.write(&batch).map_err(Error::Arrow)?;
                writer.finish().map_err(Error::Arrow)?;
            }

            // Compress with zstd
            let compressed = zstd::encode_all(ipc_buf.as_slice(), 3)
                .map_err(|e| Error::Io(io::Error::other(e)))?;

            group_blobs.push((gi as u8, compressed));
        }

        // Write chunk header
        self.out.write_all(&(self.entries_in_chunk as u32).to_le_bytes())?;
        self.out.write_all(&[group_blobs.len() as u8])?;

        // Calculate group offsets (relative to after the group table)
        let group_table_size = group_blobs.len() * 13; // 1 + 4 + 4 + 4 per entry
        let mut offset = 4 + 1 + group_table_size; // entry_count + num_groups + table

        // Write group table
        for (group_id, blob) in &group_blobs {
            self.out.write_all(&[*group_id])?;
            self.out.write_all(&(offset as u32).to_le_bytes())?;
            self.out.write_all(&(blob.len() as u32).to_le_bytes())?;
            // Uncompressed size: we don't track it exactly, store 0 for now
            self.out.write_all(&0u32.to_le_bytes())?;
            offset += blob.len();
        }

        // Write group data blobs
        for (_, blob) in &group_blobs {
            self.out.write_all(blob)?;
        }

        // Record chunk in index
        self.chunk_index.push(ChunkIndexEntry {
            offset: chunk_offset,
            entry_count: self.entries_in_chunk as u32,
        });

        // Reset columns for next chunk
        self.columns = self.header.fields.iter()
            .map(|name| {
                let ft = self.header.resolve_field_type(name);
                let dict = self.header.resolve_field_dictionary(name);
                ColBuf::new(ft, dict, self.chunk_size)
            })
            .collect();
        self.entries_in_chunk = 0;

        Ok(())
    }

    /// Flush remaining data and write the footer.
    pub fn finish(mut self) -> Result<()> {
        // Flush any remaining entries
        self.flush_chunk()?;

        // Write footer
        let footer_offset = self.out.stream_position()?;

        // Chunk index
        self.out.write_all(&(self.chunk_index.len() as u32).to_le_bytes())?;
        for chunk in &self.chunk_index {
            self.out.write_all(&chunk.offset.to_le_bytes())?;
            self.out.write_all(&chunk.entry_count.to_le_bytes())?;
        }

        // Snapshot index
        self.out.write_all(&(self.snapshot_index.len() as u32).to_le_bytes())?;
        for snap in &self.snapshot_index {
            self.out.write_all(&[snap.snapshot_type])?;
            self.out.write_all(&snap.entry_index.to_le_bytes())?;
            self.out.write_all(&snap.offset.to_le_bytes())?;
            self.out.write_all(&snap.payload_size.to_le_bytes())?;
        }

        // Total entries
        self.out.write_all(&self.total_entries.to_le_bytes())?;

        // Footer offset (last 8 bytes of file)
        self.out.write_all(&footer_offset.to_le_bytes())?;

        self.out.flush()?;
        Ok(())
    }
}
