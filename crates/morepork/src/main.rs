use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use morepork::JsonlReader;
use morepork::header::TraceHeader;

#[derive(Parser)]
#[command(name = "morepork", about = "Inspect and compare GB Trace files")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show summary information about a trace file
    Info {
        /// Trace file to inspect
        input: PathBuf,
    },
    /// Convert JSONL trace to native .morepork format
    Convert {
        /// Input JSONL file (.morepork.jsonl or -)
        input: PathBuf,
        /// Output .morepork file
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Find entries matching a condition (e.g. pc=0x0150, a=0x01)
    Query {
        /// Trace file to search
        input: PathBuf,
        /// Condition as field=value (e.g. pc=0x0150)
        #[arg(long, short)]
        r#where: Vec<String>,
        /// Max results to show (default: 10)
        #[arg(long, default_value_t = 10)]
        max: usize,
        /// Show context entries around each match
        #[arg(long, default_value_t = 0)]
        context: usize,
        /// Show the last N entries (no --where needed)
        #[arg(long)]
        last: Option<usize>,
        /// Show entries in an index range (e.g. 4650..4680)
        #[arg(long)]
        range: Option<String>,
        /// Only show these fields (comma-separated, e.g. pc,a,f,ly)
        #[arg(long)]
        fields: Option<String>,
    },
    /// Show the trace's frame boundaries
    Frames {
        /// Trace file to inspect
        input: PathBuf,
    },
    /// Render LCD frames from pixel trace data to PNG files
    Render {
        /// Trace file with pix field
        input: PathBuf,
        /// Output directory (default: current directory)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Only render specific frame numbers (1-based, comma-separated)
        #[arg(long)]
        frames: Option<String>,
    },
    /// Downsample a trace to a coarser trigger (e.g. tcycle → mcycle).
    ///
    /// Useful for inspecting an M-cycle view of a T-cycle trace without
    /// running a comparison. For missingno (which exposes `mcycle_phase`),
    /// the default keeps entries at M-cycle boundary (phase=0x00). For
    /// other T-cycle adapters without that field, the default keeps every
    /// 4th entry. Override with `--keep` for a custom condition.
    Downsample {
        /// Input trace file
        input: PathBuf,
        /// Output trace file (default: <input>.downsampled.morepork)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Target trigger (default: mcycle)
        #[arg(long, default_value = "mcycle")]
        target: String,
        /// Custom keep condition (e.g. `mcycle_phase=0x00`). Overrides defaults.
        #[arg(long)]
        keep: Option<String>,
    },
    /// Compare two trace files and report divergences
    Diff {
        /// First trace file (reference)
        trace_a: PathBuf,
        /// Trace file to compare against the reference
        trace_b: PathBuf,
        /// Only compare these fields (comma-separated, e.g. pc,a,f)
        #[arg(long)]
        fields: Option<String>,
        /// Exclude these fields from comparison (comma-separated)
        #[arg(long)]
        exclude: Option<String>,
        /// Sync mode before comparing. Modes: auto (default; skip to the
        /// family's program entry when both traces start there, else
        /// first-common-address), cartridge, pc, none, or a condition
        /// like `pc=0x0101` / `lcdc&80` (hex values).
        #[arg(long)]
        sync: Option<String>,
        /// One-line-per-field summary output
        #[arg(long)]
        summary: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Command::Info { input } => cmd_info(&input),
        Command::Convert { input, output } => cmd_convert(&input, output),
        Command::Query { input, r#where: conditions, max, context, last, range, fields } => {
            let field_filter: Option<Vec<String>> = fields
                .map(|s| s.split(',').map(|f| f.trim().to_string()).collect());
            if let Some(n) = last {
                cmd_query_last(&input, n, field_filter.as_deref())
            } else if let Some(ref range_str) = range {
                cmd_query_range(&input, range_str, field_filter.as_deref())
            } else {
                cmd_query(&input, &conditions, max, context, field_filter.as_deref())
            }
        }
        Command::Frames { input } => cmd_frames(&input),
        Command::Render { input, output, frames } => cmd_render(&input, output, frames),
        Command::Downsample { input, output, target, keep } => cmd_downsample(&input, output, &target, keep.as_deref()),
        Command::Diff {
            trace_a,
            trace_b,
            fields,
            exclude,
            sync,
            summary,
        } => cmd_diff(&trace_a, &trace_b, fields, exclude, sync.as_deref(), summary),
    };
    process::exit(code);
}

// ---------------------------------------------------------------------------
// info
// ---------------------------------------------------------------------------

fn cmd_info(path: &PathBuf) -> i32 {
    let store = match morepork::store::open_trace_store(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };

    let h = store.header();
    println!("File:      {}", path.display());
    println!("Emulator:  {}", h.emulator);
    println!("Version:   {}", h.emulator_version);
    println!("Family:    {}", h.family_def().id);
    println!("Model:     {}", h.model);
    println!("Profile:   {}", h.profile);
    println!("Trigger:   {:?}", h.trigger);
    println!("Boot ROM:  {}", format_boot_rom(&h.boot_rom));
    println!("ROM hash:  {}", h.rom_sha256);
    println!("Fields:    {}", h.fields.join(", "));

    let count = store.entry_count();
    println!("Entries:   {count}");

    let boundaries = store.frame_boundaries();
    if !boundaries.is_empty() {
        println!("Frames:    {}", boundaries.len());
    }

    if let Ok(meta) = std::fs::metadata(path) {
        let size = meta.len();
        println!("File size: {size} bytes ({:.1} MB)", size as f64 / 1024.0 / 1024.0);
    }

    0
}

// ---------------------------------------------------------------------------
// frames
// ---------------------------------------------------------------------------

fn cmd_frames(path: &PathBuf) -> i32 {
    let store = match morepork::store::open_trace_store(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };

    let boundaries = store.frame_boundaries();
    if boundaries.is_empty() {
        println!("No frames detected (trace has no frame boundaries)");
        return 0;
    }

    let total = store.entry_count();
    println!("Frames: {}", boundaries.len());
    println!("Entries: {total}");
    println!();

    for (i, &start) in boundaries.iter().enumerate() {
        let start = start as usize;
        let end = if i + 1 < boundaries.len() {
            boundaries[i + 1] as usize
        } else {
            total
        };
        let size = end - start;
        println!("  Frame {:>3}  entries {:>8}..{:<8}  ({} entries)", i + 1, start, end, size);
    }

    0
}

// ---------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------

fn cmd_render(path: &PathBuf, output_dir: Option<PathBuf>, frame_filter: Option<String>) -> i32 {
    let store = match morepork::store::open_trace_store(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };

    // Indexed frame snapshots render family-agnostically: each payload
    // carries its own dimensions and palette.
    if store.header().pix_format == morepork::header::PixFormat::Indexed8 {
        return render_indexed_frames(store.as_ref(), path, output_dir, frame_filter);
    }

    let family = store.header().family_def();
    if family.id != "gb" {
        eprintln!("Error: frame rendering is not implemented for family '{}'", family.id);
        return 1;
    }
    let frames = morepork::family::gb::framebuffer::reconstruct_frames(store.as_ref());
    if frames.is_empty() {
        eprintln!("No frames with pixel data found (trace needs a 'pix' field)");
        return 1;
    }

    let out_dir = output_dir.unwrap_or_else(|| PathBuf::from("."));
    if !out_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            eprintln!("Failed to create output directory: {e}");
            return 1;
        }
    }

    // Parse frame filter
    let selected: Option<Vec<usize>> = frame_filter.map(|s| {
        s.split(',')
            .filter_map(|n| n.trim().parse::<usize>().ok())
            .collect()
    });

    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("frame");

    for frame in &frames {
        let frame_num = frame.index + 1; // 1-based for display
        if let Some(ref sel) = selected {
            if !sel.contains(&frame_num) { continue; }
        }

        let png_data = frame.to_png();
        let out_path = out_dir.join(format!("{stem}_frame{frame_num:03}.png"));
        match std::fs::write(&out_path, &png_data) {
            Ok(_) => {
                let pix_count: usize = frame.pixels.iter().filter(|&&p| p > 0).count();
                println!("  Frame {:>3}  {} ({} non-zero pixels)",
                    frame_num, out_path.display(), pix_count);
            }
            Err(e) => {
                eprintln!("  Frame {:>3}  ERROR: {e}", frame_num);
            }
        }
    }

    println!("Rendered {} frame(s)", frames.len());
    0
}

/// Render `frame` snapshot payloads (`snapshot::IndexedFrame`) to PNGs.
fn render_indexed_frames(
    store: &dyn morepork::store::TraceStore,
    path: &PathBuf,
    output_dir: Option<PathBuf>,
    frame_filter: Option<String>,
) -> i32 {
    let count = store.frame_boundaries().len();
    if count == 0 {
        eprintln!("No frame snapshots found");
        return 1;
    }

    let out_dir = output_dir.unwrap_or_else(|| PathBuf::from("."));
    if !out_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            eprintln!("Failed to create output directory: {e}");
            return 1;
        }
    }

    let selected: Option<Vec<usize>> = frame_filter.map(|s| {
        s.split(',')
            .filter_map(|n| n.trim().parse::<usize>().ok())
            .collect()
    });
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("frame");

    let mut rendered = 0;
    for idx in 0..count {
        let frame_num = idx + 1; // 1-based for display
        if let Some(ref sel) = selected {
            if !sel.contains(&frame_num) { continue; }
        }
        let Some(payload) = store.frame_payload(idx) else {
            println!("  Frame {frame_num:>3}  (no pixel payload)");
            continue;
        };
        let Some(frame) = morepork::snapshot::IndexedFrame::from_bytes(&payload) else {
            eprintln!("  Frame {frame_num:>3}  ERROR: malformed indexed-frame payload");
            continue;
        };

        let rgba = frame.to_rgba();
        let mut png_data = Vec::new();
        {
            let mut encoder =
                png::Encoder::new(&mut png_data, frame.width as u32, frame.height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&rgba).unwrap();
        }

        let out_path = out_dir.join(format!("{stem}_frame{frame_num:03}.png"));
        match std::fs::write(&out_path, &png_data) {
            Ok(_) => {
                println!(
                    "  Frame {:>3}  {} ({}x{})",
                    frame_num,
                    out_path.display(),
                    frame.width,
                    frame.height
                );
                rendered += 1;
            }
            Err(e) => {
                eprintln!("  Frame {frame_num:>3}  ERROR: {e}");
            }
        }
    }

    println!("Rendered {rendered} frame(s)");
    0
}

// ---------------------------------------------------------------------------
// convert
// ---------------------------------------------------------------------------

fn cmd_convert(input: &PathBuf, output: Option<PathBuf>) -> i32 {
    let is_stdin = input.as_os_str() == "-";

    let output = match output {
        Some(o) => o,
        None if is_stdin => {
            eprintln!("Error: --output required when reading from stdin");
            return 1;
        }
        None => {
            let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("trace");
            let stem = stem.strip_suffix(".morepork").unwrap_or(stem);
            input.with_file_name(format!("{stem}.morepork"))
        }
    };

    let reader = if is_stdin {
        use std::io::BufReader;
        match morepork::JsonlReader::from_reader(BufReader::new(std::io::stdin())) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Error reading stdin: {e}");
                return 1;
            }
        }
    } else {
        match JsonlReader::open(input) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Error opening input: {e}");
                return 1;
            }
        }
    };

    let header = reader.header().clone();
    convert_to_morepork(reader, &output, &header)
}

fn convert_to_morepork(
    reader: JsonlReader,
    output: &PathBuf,
    header: &TraceHeader,
) -> i32 {
    use morepork::format::write::MoreporkWriter;
    use morepork::profile::FieldType;

    let mut writer = match MoreporkWriter::create(output, header, &[]) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("Error creating output: {e}");
            return 1;
        }
    };

    let mut count: u64 = 0;

    for result in reader {
        match result {
            Ok(entry) => {
                // Set all field values from the entry
                for (col, name) in header.fields.iter().enumerate() {
                    let val = entry.get(name);
                    let ft = header.resolve_field_type(name);
                    let nullable = header.resolve_field_nullable(name);

                    if nullable && val.is_none() {
                        writer.set_null(col);
                        continue;
                    }

                    match ft {
                        FieldType::UInt64 => {
                            writer.set_u64(col, val.and_then(|v| v.as_u64()).unwrap_or(0));
                        }
                        FieldType::UInt16 => {
                            let v = val
                                .and_then(|v| v.as_u64().or_else(|| {
                                    v.as_str().and_then(|s| {
                                        let s = s.strip_prefix("0x").unwrap_or(s);
                                        u64::from_str_radix(s, 16).ok()
                                    })
                                }))
                                .unwrap_or(0) as u16;
                            if nullable && v == 0 { writer.set_null(col); }
                            else { writer.set_u16(col, v); }
                        }
                        FieldType::UInt8 => {
                            let v = val
                                .and_then(|v| v.as_u64().or_else(|| {
                                    v.as_str().and_then(|s| {
                                        let s = s.strip_prefix("0x").unwrap_or(s);
                                        u64::from_str_radix(s, 16).ok()
                                    })
                                }))
                                .unwrap_or(0) as u8;
                            if nullable && v == 0 { writer.set_null(col); }
                            else { writer.set_u8(col, v); }
                        }
                        FieldType::Bool => {
                            writer.set_bool(col, val.and_then(|v| v.as_bool()).unwrap_or(false));
                        }
                        FieldType::Str => {
                            let s = val.and_then(|v| v.as_str()).unwrap_or("");
                            if nullable && s.is_empty() { writer.set_null(col); }
                            else { writer.set_str(col, s); }
                        }
                    }
                }

                if let Err(e) = writer.finish_entry() {
                    eprintln!("Error writing entry {count}: {e}");
                    return 1;
                }

                // Spec: `_frame: true` marks a frame boundary at the current entry.
                let frame_boundary = entry
                    .get("_frame")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if frame_boundary {
                    if let Err(e) = writer.mark_frame(None) {
                        eprintln!("Error marking frame at entry {count}: {e}");
                        return 1;
                    }
                }
                count += 1;
            }
            Err(e) => {
                eprintln!("Error reading entry {count}: {e}");
                return 1;
            }
        }
    }

    if let Err(e) = writer.finish() {
        eprintln!("Error finalizing: {e}");
        return 1;
    }

    let output_size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    println!("Converted {count} entries to {} ({output_size} bytes)", output.display());
    0
}

// ---------------------------------------------------------------------------
// downsample
// ---------------------------------------------------------------------------

fn cmd_downsample(input: &PathBuf, output: Option<PathBuf>, target: &str, keep: Option<&str>) -> i32 {
    use morepork::format::write::MoreporkWriter;
    use morepork::header::Trigger;
    use morepork::profile::FieldType;

    let target_trigger = match target.to_lowercase().as_str() {
        "instruction" => Trigger::Instruction,
        "mcycle" => Trigger::Mcycle,
        "tcycle" => Trigger::Tcycle,
        "cycle" => Trigger::Cycle,
        "scanline" => Trigger::Scanline,
        "frame" => Trigger::Frame,
        "custom" => Trigger::Custom,
        other => { eprintln!("Error: unknown target trigger '{other}'"); return 1; }
    };

    let store = match morepork::store::open_trace_store(input) {
        Ok(s) => s,
        Err(e) => { eprintln!("Error opening input: {e}"); return 1; }
    };

    // Pick the keep filter. Explicit --keep wins; otherwise default by input trigger.
    enum Filter {
        Condition(morepork::query::Condition),
        EveryNth(usize),
    }
    let filter = if let Some(cond_str) = keep {
        match morepork::query::parse_condition(cond_str, store.header().family_def()) {
            Ok(c) => Filter::Condition(c),
            Err(e) => { eprintln!("Error: bad --keep condition: {e}"); return 1; }
        }
    } else {
        // Default for tcycle → mcycle: mcycle_phase=0x00 if field exists, else every 4th.
        let input_is_tcycle = matches!(store.header().trigger, Trigger::Tcycle);
        let target_is_mcycle = matches!(target_trigger, Trigger::Mcycle);
        if input_is_tcycle && target_is_mcycle {
            if store.has_field("mcycle_phase") {
                // Missingno's mcycle_phase ring counter is (in order within one
                // M-cycle) 0x0E → 0x07 → 0x01 → 0x08. Phase 0x0E is the first
                // T-cycle of an M-cycle — picking it gives one entry per M-cycle
                // at the "after previous M-cycle's commits" sample point, which
                // is the natural alignment for an M-cycle-cadence trace.
                match morepork::query::parse_condition("mcycle_phase=0x0e", store.header().family_def()) {
                    Ok(c) => Filter::Condition(c),
                    Err(e) => { eprintln!("Error: internal default condition failed: {e}"); return 1; }
                }
            } else {
                Filter::EveryNth(4)
            }
        } else {
            eprintln!("Error: no default filter for {:?} → {:?}; pass --keep <condition>",
                store.header().trigger, target_trigger);
            return 1;
        }
    };

    // Build the output header by cloning the input's and overriding trigger.
    let mut out_header = store.header().clone();
    out_header.trigger = target_trigger.clone();

    let output = output.unwrap_or_else(|| {
        let mut p = input.clone();
        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "trace".to_string());
        p.set_file_name(format!("{stem}.downsampled.morepork"));
        p
    });

    let mut writer = match MoreporkWriter::create(&output, &out_header, &[]) {
        Ok(w) => w,
        Err(e) => { eprintln!("Error creating output: {e}"); return 1; }
    };

    // Frame boundaries from input — map old entry index → emit a mark_frame
    // at the first kept entry on/after that index.
    let frame_in_indices: Vec<u32> = store.frame_boundaries();
    let mut frame_cursor = 0usize;

    let total = store.entry_count();
    let mut kept = 0u64;

    for i in 0..total {
        // Always include entry 0 so the trace start is preserved. T-cycle
        // adapters that begin capture partway through an M-cycle (e.g.
        // missingno's step_tcycle takes 2 phases before its first capture)
        // leave the first M-cycle missing the canonical "boundary" phase the
        // filter looks for. Including entry 0 unconditionally keeps the first
        // visible PC in the downsampled trace.
        let keep_this = i == 0 || match &filter {
            Filter::Condition(c) => store.eval_condition_trait(c, i),
            Filter::EveryNth(n) => i % n == (n - 1),
        };
        if !keep_this { continue; }

        // Copy all fields for this row into the writer.
        for (col, name) in out_header.fields.iter().enumerate() {
            let ft = out_header.resolve_field_type(name);
            let nullable = out_header.resolve_field_nullable(name);
            let in_col = match store.field_col(name) { Some(c) => c, None => { writer.set_null(col); continue; } };

            if nullable && store.is_null(in_col, i) { writer.set_null(col); continue; }

            match ft {
                FieldType::UInt64 => writer.set_u64(col, store.get_numeric(in_col, i)),
                FieldType::UInt16 => writer.set_u16(col, store.get_numeric(in_col, i) as u16),
                FieldType::UInt8  => writer.set_u8(col, store.get_numeric(in_col, i) as u8),
                FieldType::Bool   => writer.set_bool(col, store.get_bool(in_col, i)),
                FieldType::Str    => writer.set_str(col, &store.get_str(in_col, i)),
            }
        }

        if let Err(e) = writer.finish_entry() {
            eprintln!("Error writing entry {kept}: {e}"); return 1;
        }

        // Emit any frame boundaries from input that fall at or before this row.
        while frame_cursor < frame_in_indices.len()
            && (frame_in_indices[frame_cursor] as usize) <= i
        {
            if let Err(e) = writer.mark_frame(None) {
                eprintln!("Error marking frame: {e}"); return 1;
            }
            frame_cursor += 1;
        }

        kept += 1;
    }

    if let Err(e) = writer.finish() {
        eprintln!("Error finalizing: {e}"); return 1;
    }

    let out_size = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    println!("Downsampled {total} entries → {kept} ({} bytes) at {}",
        out_size, output.display());
    0
}

// ---------------------------------------------------------------------------
fn cmd_query(input: &PathBuf, conditions: &[String], max: usize, context: usize, field_filter: Option<&[String]>) -> i32 {
    if conditions.is_empty() {
        eprintln!("Error: at least one --where condition required");
        return 1;
    }

    let store = match morepork::store::open_trace_store(input) {
        Ok(s) => s,
        Err(e) => { eprintln!("Error: {e}"); return 1; }
    };

    let fields = store.header().fields.clone();

    // Use the store's query_range for the first condition, then filter
    let condition_str = conditions.join(" AND ");
    let matches = match store.query_range(&condition_str, 0, store.entry_count()) {
        Ok(m) => m,
        Err(e) => { eprintln!("Error: {e}"); return 1; }
    };

    let matches_found = matches.len();
    let displayed_matches = matches_found.min(max);

    for (display_idx, &entry_idx) in matches.iter().enumerate() {
        if display_idx >= max { break; }
        let i = entry_idx as usize;

        if display_idx > 0 { println!("  ---"); }

        // Context before
        if context > 0 {
            let ctx_start = if i >= context { i - context } else { 0 };
            for ci in ctx_start..i {
                print!("  [{ci}]");
                print_store_entry(&*store, ci, &fields, field_filter);
                println!();
            }
        }

        // The match
        print!("> [{i}]");
        print_store_entry(&*store, i, &fields, field_filter);
        println!();

        // Context after
        if context > 0 {
            let ctx_end = (i + context + 1).min(store.entry_count());
            for ci in (i + 1)..ctx_end {
                print!("  [{ci}]");
                print_store_entry(&*store, ci, &fields, field_filter);
                println!();
            }
        }
    }

    // Mimic the old output format
    println!("\n{matches_found} match(es) found.");
    if displayed_matches < matches_found {
        println!("  (showing first {displayed_matches}, use --max to see more)");
    }

    0
}

fn cmd_query_range(input: &PathBuf, range_str: &str, field_filter: Option<&[String]>) -> i32 {
    let store = match morepork::store::open_trace_store(input) {
        Ok(s) => s,
        Err(e) => { eprintln!("Error: {e}"); return 1; }
    };

    let fields = store.header().fields.clone();
    let total = store.entry_count();

    // Parse "start..end" range
    let parts: Vec<&str> = range_str.split("..").collect();
    if parts.len() != 2 {
        eprintln!("Error: range must be in format START..END (e.g. 4650..4680)");
        return 1;
    }
    let start: usize = match parts[0].parse() {
        Ok(n) => n,
        Err(_) => { eprintln!("Error: invalid range start '{}'", parts[0]); return 1; }
    };
    let end: usize = match parts[1].parse::<usize>() {
        Ok(n) => n.min(total),
        Err(_) => { eprintln!("Error: invalid range end '{}'", parts[1]); return 1; }
    };

    if start >= total {
        eprintln!("Error: range start {start} exceeds trace length {total}");
        return 1;
    }

    for i in start..end {
        print!("  [{i}]");
        print_store_entry(&*store, i, &fields, field_filter);
        println!();
    }

    0
}

fn cmd_query_last(input: &PathBuf, n: usize, field_filter: Option<&[String]>) -> i32 {
    let store = match morepork::store::open_trace_store(input) {
        Ok(s) => s,
        Err(e) => { eprintln!("Error: {e}"); return 1; }
    };

    let fields = store.header().fields.clone();
    let total = store.entry_count();
    let start = total.saturating_sub(n);

    for i in start..total {
        print!(" ");
        print_store_entry(&*store, i, &fields, field_filter);
        println!();
    }

    0
}

fn print_store_entry(
    store: &dyn morepork::store::TraceStore,
    row: usize,
    fields: &[String],
    field_filter: Option<&[String]>,
) {
    use morepork::profile::FieldType;
    let header = store.header();
    for (col, name) in fields.iter().enumerate() {
        if let Some(filter) = field_filter {
            if !filter.iter().any(|f| f == name) { continue; }
        }
        if store.is_null(col, row) { continue; }
        let ft = header.resolve_field_type(name);
        match ft {
            FieldType::Bool => {
                let v = store.get_bool(col, row);
                print!(" {name}={v}");
            }
            FieldType::Str => {
                let v = store.get_str(col, row);
                if !v.is_empty() { print!(" {name}={v}"); }
            }
            _ => {
                let v = store.get_numeric(col, row);
                print!(" {name}={v:02x}");
            }
        }
    }
}

fn format_boot_rom(boot_rom: &morepork::BootRom) -> String {
    match boot_rom {
        morepork::BootRom::Skip => "skip".to_string(),
        morepork::BootRom::Builtin => "builtin".to_string(),
        morepork::BootRom::Stripped(orig) => format!("stripped:{orig}"),
        morepork::BootRom::Sha256(s) => s.clone(),
    }
}


// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

fn cmd_diff(
    path_a: &PathBuf,
    path_b: &PathBuf,
    fields_filter: Option<String>,
    exclude_filter: Option<String>,
    sync: Option<&str>,
    summary: bool,
) -> i32 {
    use morepork::comparison::TraceComparison;

    let store_a = match morepork::store::open_trace_store(path_a) {
        Ok(s) => s,
        Err(e) => { eprintln!("Error opening {}: {e}", path_a.display()); return 1; }
    };
    let store_b = match morepork::store::open_trace_store(path_b) {
        Ok(s) => s,
        Err(e) => { eprintln!("Error opening {}: {e}", path_b.display()); return 1; }
    };

    let mut comp = match TraceComparison::align(&*store_a, &*store_b, sync) {
        Ok(c) => c,
        Err(e) => { eprintln!("Error aligning traces: {e}"); return 1; }
    };

    let name_a = &store_a.header().emulator;
    let name_b = &store_b.header().emulator;

    // Determine which fields to compare
    let fields_a = &store_a.header().fields;
    let fields_b = &store_b.header().fields;
    let common_fields: Vec<String> = fields_a.iter()
        .filter(|f| fields_b.contains(f))
        .cloned()
        .collect();

    let include: Option<Vec<String>> = fields_filter
        .map(|s| s.split(',').map(String::from).collect());
    let exclude: Option<Vec<String>> = exclude_filter
        .map(|s| s.split(',').map(String::from).collect());

    let fields_to_compare: Vec<&str> = common_fields.iter()
        .filter(|f| {
            if let Some(ref inc) = include {
                if !inc.iter().any(|i| i == *f) { return false; }
            }
            if let Some(ref exc) = exclude {
                if exc.iter().any(|e| e == *f) { return false; }
            }
            true
        })
        .map(|s| s.as_str())
        .collect();

    // Compute stats via TraceComparison with field filter
    let stats = comp.compute_stats_filtered(Some(&fields_to_compare)).to_vec();
    let aligned_count = comp.len();

    let relevant_stats: Vec<_> = stats.iter().collect();

    let total_diffs: usize = relevant_stats.iter().map(|s| s.diff_count).sum();
    let is_identical = total_diffs == 0;

    if summary {
        let total_m: usize = relevant_stats.iter().map(|s| s.match_count).sum();
        let total_d: usize = relevant_stats.iter().map(|s| s.diff_count).sum();
        let pct = if total_m + total_d > 0 { total_m as f64 / (total_m + total_d) as f64 * 100.0 } else { 100.0 };
        println!("{name_a} vs {name_b}: {} aligned entries, {:.1}% match",
            aligned_count, pct);
        for s in &relevant_stats {
            if s.diff_count > 0 {
                // Find first diff for this field
                let first_idx = (0..comp.len())
                    .find(|&i| comp.field_differs(&s.name, i))
                    .unwrap_or(0);
                let row_a = comp.original_a(first_idx);
                let row_b = comp.original_b(first_idx);
                let col_a = store_a.field_col(&s.name).unwrap();
                let col_b = store_b.field_col(&s.name).unwrap();
                let val_a = store_a.get_numeric(col_a, row_a);
                let val_b = store_b.get_numeric(col_b, row_b);
                println!("  {:<16} {:>8} diffs, first at idx={}: {}={:02x}  {}={:02x}",
                    s.name, s.diff_count, first_idx,
                    name_a, val_a, name_b, val_b);
            }
        }
    } else {
        println!("Comparing: {} vs {}", name_a, name_b);
        println!("  Aligned entries: {}", aligned_count);
        println!("  Store A entries: {}", store_a.entry_count());
        println!("  Store B entries: {}", store_b.entry_count());

        // Fields only in one trace
        let only_a: Vec<&str> = fields_a.iter()
            .filter(|f| !fields_b.contains(f))
            .map(|s| s.as_str())
            .collect();
        let only_b: Vec<&str> = fields_b.iter()
            .filter(|f| !fields_a.contains(f))
            .map(|s| s.as_str())
            .collect();
        if !only_a.is_empty() {
            println!("  Fields only in {}: {}", name_a, only_a.join(", "));
        }
        if !only_b.is_empty() {
            println!("  Fields only in {}: {}", name_b, only_b.join(", "));
        }

        println!();

        if is_identical {
            println!("  IDENTICAL ({} common fields, {} entries)", fields_to_compare.len(), aligned_count);
        } else {
            println!("  Divergences:");
            for s in &relevant_stats {
                let pct = s.match_pct();
                if s.diff_count > 0 {
                    println!("    {:<16} {:>8} diffs ({:.1}% match)", s.name, s.diff_count, pct);
                }
            }

            // Show first few divergent entries
            println!();
            println!("  First divergences:");
            let mut shown = 0;
            for i in 0..comp.len() {
                let any_diff = fields_to_compare.iter().any(|f| comp.field_differs(f, i));
                if any_diff {
                    let row_a = comp.original_a(i);
                    let row_b = comp.original_b(i);
                    print!("    [{i}] ");
                    for f in &fields_to_compare {
                        if comp.field_differs(f, i) {
                            let col_a = store_a.field_col(f).unwrap();
                            let col_b = store_b.field_col(f).unwrap();
                            let va = store_a.get_numeric(col_a, row_a);
                            let vb = store_b.get_numeric(col_b, row_b);
                            print!("{f}:{:02x}|{:02x} ", va, vb);
                        }
                    }
                    println!();
                    shown += 1;
                    if shown >= 10 { break; }
                }
            }
        }
    }

    if is_identical { 0 } else { 1 }
}
