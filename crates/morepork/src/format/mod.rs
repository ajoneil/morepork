//! Native `.morepork` binary format.
//!
//! File layout (v2):
//! ```text
//! [Magic "MPRK" (4)] [Version (1)] [Header len (4)] [Header JSON zstd]
//! [Snapshot record]*  -- initial snapshots (optional, before any chunks)
//! [Chunk | Snapshot record]*  -- interleaved chunks and inline snapshots
//! [Footer]
//! [Footer offset (8)]
//! ```
//!
//! Each chunk contains ~64K entries with field groups compressed independently.
//!
//! Snapshot records are typed, self-describing blobs that carry bulk state
//! at specific points in the trace stream. They serve two purposes:
//! - Frame boundaries (with optional screen data)
//! - Initial state (memory, APU state, etc.)
//!
//! A snapshot record on disk:
//! ```text
//! [Tag "SNAP" (4)] [Type (1)] [Entry index (8)] [Payload len (4)] [Payload zstd]
//! ```

pub mod write;
pub mod read;
pub mod convert;

pub const MAGIC: &[u8; 4] = b"MPRK";
pub const VERSION: u8 = 2;
pub const SNAPSHOT_TAG: &[u8; 4] = b"SNAP";

/// Default maximum entries per chunk.
pub const DEFAULT_CHUNK_SIZE: usize = 65536;

/// Snapshot tag for frame boundaries. The payload is optional screen
/// data: raw GB pixels in the header's pix_format, or a serialized
/// `snapshot::IndexedFrame`.
pub const TAG_FRAME: u8 = 0;

/// Snapshot tag for bulk memory contents.
pub const TAG_MEMORY: u8 = 1;

/// First tag available to console families. `frame` and `memory` are the
/// only kinds the format itself defines; a family claims tags from here
/// up (its registry entry lists their kind names, which the writer
/// records in the header's `snapshot_kinds`).
pub const FAMILY_TAG_BASE: u8 = 2;

/// A field group definition — maps a group name to its column indices.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldGroup {
    pub name: String,
    pub fields: Vec<String>,
}

/// Per-chunk statistics for a single numeric field.
#[derive(Debug, Clone, Default)]
pub struct FieldStats {
    pub min: u64,
    pub max: u64,
}

/// Entry in the chunk index (footer).
#[derive(Debug, Clone)]
pub struct ChunkIndexEntry {
    /// Byte offset of the chunk from file start.
    pub offset: u64,
    /// Number of entries in this chunk.
    pub entry_count: u32,
}

/// Entry in the snapshot index (footer).
#[derive(Debug, Clone)]
pub struct SnapshotIndexEntry {
    /// Snapshot type.
    pub snapshot_type: u8,
    /// Global entry index at which this snapshot occurs.
    pub entry_index: u64,
    /// Byte offset of the snapshot record from file start.
    pub offset: u64,
    /// Size of the compressed payload (0 = no payload).
    pub payload_size: u32,
}

/// The footer, read from the end of the file.
#[derive(Debug, Clone)]
pub struct Footer {
    pub chunks: Vec<ChunkIndexEntry>,
    pub snapshots: Vec<SnapshotIndexEntry>,
    pub total_entries: u64,
}

