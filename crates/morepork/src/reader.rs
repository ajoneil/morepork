use crate::entry::TraceEntry;
use crate::error::{Error, Result};
use crate::header::TraceHeader;
use flate2::read::GzDecoder;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

/// Streaming reader for `.morepork.jsonl` and `.morepork.jsonl.gz` files.
///
/// Reads entries one at a time — never loads the full file into memory.
/// The header line is
/// optional: if the first JSON object lacks `"_header": true` it is treated
/// as the first data entry and a default header is synthesised. Header fields
/// other than `_header` all have defaults, and `fields` is inferred from the
/// first data line's keys when not supplied.
pub struct JsonlReader {
    lines: Box<dyn BufRead>,
    header: TraceHeader,
    /// First data entry, when the input was headerless or the header lacked a
    /// `fields` list (in which case we had to peek a data line to infer them).
    /// Drained by the first `next_entry` call.
    pending_first_entry: Option<TraceEntry>,
}

impl JsonlReader {
    /// Open a trace file and read its header.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)?;

        let reader: Box<dyn Read> = if path.extension().is_some_and(|ext| ext == "gz") {
            Box::new(GzDecoder::new(file))
        } else {
            Box::new(file)
        };

        let lines = BufReader::with_capacity(64 * 1024, reader);
        Self::from_reader(lines)
    }

    /// Create a reader from any buffered reader (e.g. stdin).
    /// Skips any non-JSON lines before the first JSON object (e.g. emulator debug output).
    pub fn from_reader<R: BufRead + 'static>(mut reader: R) -> Result<Self> {
        // Find the first line that looks like JSON, skipping any preceding debug noise.
        let first_value = loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                return Err(Error::MissingHeader);
            }
            if line.trim().starts_with('{') {
                break serde_json::from_str::<serde_json::Value>(line.trim())?;
            }
        };

        let is_header = first_value
            .get("_header")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let (mut header, pending_first_entry) = if is_header {
            let header: TraceHeader = serde_json::from_value(first_value)?;
            (header, None)
        } else {
            // Headerless: synthesise a default header, keep the data entry for the
            // first `next_entry` call. Field list is inferred below.
            let header = TraceHeader { _header: true, ..Default::default() };
            let entry = TraceEntry::from_json_value(&first_value).ok_or_else(|| {
                Error::InvalidHeader("first JSON object is not a data entry".into())
            })?;
            (header, Some(entry))
        };

        // If the header didn't supply fields, infer them — from the buffered
        // first entry if we have one, otherwise by peeking the next line.
        let pending_first_entry = if header.fields.is_empty() {
            let (peeked_entry, source) = match pending_first_entry {
                Some(entry) => (entry, true),
                None => {
                    // Peek the next non-empty line as a data entry.
                    let entry = loop {
                        let mut line = String::new();
                        let n = reader.read_line(&mut line)?;
                        if n == 0 { break None; }
                        let trimmed = line.trim();
                        if trimmed.is_empty() { continue; }
                        let value: serde_json::Value = serde_json::from_str(trimmed)?;
                        break TraceEntry::from_json_value(&value);
                    };
                    match entry {
                        Some(e) => (e, true),
                        None => {
                            // No data lines — leave fields empty, no entries to read.
                            header.validate()?;
                            return Ok(Self {
                                lines: Box::new(reader),
                                header,
                                pending_first_entry: None,
                            });
                        }
                    }
                }
            };
            header.fields = infer_fields(&peeked_entry);
            if source { Some(peeked_entry) } else { None }
        } else {
            pending_first_entry
        };

        header.validate()?;
        Ok(Self {
            lines: Box::new(reader),
            header,
            pending_first_entry,
        })
    }

    /// Get a reference to the parsed header.
    pub fn header(&self) -> &TraceHeader {
        &self.header
    }

    /// Read the next trace entry, or `None` at end of file.
    pub fn next_entry(&mut self) -> Result<Option<TraceEntry>> {
        if let Some(entry) = self.pending_first_entry.take() {
            return Ok(Some(entry));
        }

        let mut line = String::new();
        let bytes_read = self.lines.read_line(&mut line)?;
        if bytes_read == 0 {
            return Ok(None);
        }

        let line = line.trim();
        if line.is_empty() {
            return Ok(None);
        }

        let value: serde_json::Value = serde_json::from_str(line)?;
        TraceEntry::from_json_value(&value).ok_or_else(|| {
            Error::InvalidHeader("entry is not a JSON object".into())
        })
        .map(Some)
    }
}

/// Collect ordinary data-field names from an entry, skipping spec-defined
/// control fields (`_header`, `_frame`, `framebuffer`).
fn infer_fields(entry: &TraceEntry) -> Vec<String> {
    let json = entry.to_json_value();
    let Some(obj) = json.as_object() else { return Vec::new() };
    obj.keys()
        .filter(|k| !k.starts_with('_') && *k != "framebuffer")
        .cloned()
        .collect()
}

/// Iterator adapter over trace entries.
impl Iterator for JsonlReader {
    type Item = Result<TraceEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_entry() {
            Ok(Some(entry)) => Some(Ok(entry)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}
