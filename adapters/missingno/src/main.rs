use std::fs;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process;

use clap::Parser;
use missingno_gb::cartridge::Cartridge;
use missingno_gb::execute::{PhaseResult, StepResult};
use missingno_gb::trace::{BootRom, Profile, Traceable, Tracer, Trigger};
use missingno_gb::GameBoy;
use missingno_gbc::GameBoyColor;

#[derive(Parser)]
#[command(name = "gbtrace-missingno")]
struct Args {
    #[arg(long)]
    rom: PathBuf,

    #[arg(long)]
    profile: PathBuf,

    #[arg(long)]
    output: PathBuf,

    #[arg(long, default_value_t = 3000)]
    frames: u32,

    /// Run for exactly N T-cycles, then capture the screen and stop (gambatte
    /// tests: read the framebuffer after a fixed cycle budget, not N vblanks).
    #[arg(long)]
    until_tcycle: Option<u64>,

    /// Console model: dmg (original Game Boy) or cgb (Game Boy Color).
    #[arg(long, default_value = "dmg")]
    model: String,

    /// Stop when opcode at PC matches (hex, e.g. 40 for LD B,B)
    #[arg(long, value_parser = parse_hex_u8)]
    stop_opcode: Option<u8>,

    /// Stop when this byte is sent via serial (hex, e.g. 0A)
    #[arg(long, value_parser = parse_hex_u8)]
    stop_on_serial: Option<u8>,

    /// Number of serial byte matches before stopping
    #[arg(long, default_value_t = 1)]
    stop_serial_count: u32,

    /// Reference .pix file for screenshot matching
    #[arg(long)]
    reference: Option<PathBuf>,

    /// Extra frames to capture after stop condition
    #[arg(long, default_value_t = 0)]
    extra_frames: u32,

    /// Report last-frame audio activity to stderr as `AUDIO=0` / `AUDIO=1`
    /// (used by the gambatte `_outaudio` pass/fail check).
    #[arg(long, default_value_t = false)]
    report_audio: bool,

    /// Stop when memory ADDR equals VAL (hex, e.g. FF82=01) or ADDR!=VAL. Can be repeated.
    #[arg(long = "stop-when", value_parser = parse_stop_when)]
    stop_when: Vec<StopWhen>,
}

#[derive(Clone)]
struct StopWhen {
    addr: u16,
    value: u8,
    negate: bool,
}

fn parse_hex_u8(s: &str) -> Result<u8, String> {
    u8::from_str_radix(s, 16).map_err(|e| format!("invalid hex byte: {e}"))
}

fn parse_stop_when(s: &str) -> Result<StopWhen, String> {
    let (addr_s, val_s, negate) = if let Some((a, v)) = s.split_once("!=") {
        (a, v, true)
    } else if let Some((a, v)) = s.split_once('=') {
        (a, v, false)
    } else {
        return Err("expected ADDR=VAL or ADDR!=VAL (e.g. A000!=80)".to_string());
    };
    let addr = u16::from_str_radix(addr_s, 16).map_err(|e| format!("invalid address: {e}"))?;
    let value = u8::from_str_radix(val_s, 16).map_err(|e| format!("invalid value: {e}"))?;
    Ok(StopWhen { addr, value, negate })
}

/// Reference screenshots are raw RGB555 (160×144×3 bytes, each channel 0-31).
/// Comparing at the CGB's native 5-bit precision is expansion-neutral.
fn load_reference(path: &PathBuf) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| panic!("Failed to read reference {}: {e}", path.display()))
}

/// DMG shade index (0=lightest) → greyscale RGB555 channel value.
const GREY555: [u8; 4] = [31, 21, 10, 0];

/// Per-channel RGB555 match with a small tolerance, to absorb minor
/// 555→888 expansion / quantisation differences between emulators.
fn rgb555_match(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (*x as i16 - *y as i16).abs() <= 1)
}

/// The run loop needs a few operations beyond [`Traceable`]; this trait
/// abstracts over `GameBoy` (DMG) and `GameBoyColor` (CGB) so a single
/// generic loop traces both.
//
// NOTE: the local trait methods below are deliberately named *differently* from
// missingno's inherent `Console<M>` methods they delegate to (e.g. `observe_tcycle`
// → `execute_tcycle_observed`, `step_instr` → `step`). A same-named method (as the
// former `step_phase` was) resolves to this trait rather than the inherent method
// the moment upstream renames/removes the inherent one — an infinite self-recursion
// that release-mode tail-call-optimises into a silent hang. Keeping the names
// distinct makes that trap impossible.
trait Console: Traceable {
    /// Advance one CPU T-cycle, invoking `after` after each master edge (rise
    /// then fall) with that edge's [`PhaseResult`] — the gbtrace capture hook.
    /// Returns whether a new frame was produced. A `Break` from the rise's
    /// observer defers the fall to the next call (double-speed mid-pair retire).
    fn observe_tcycle(
        &mut self,
        after: impl FnMut(&mut Self, &PhaseResult) -> ControlFlow<()>,
    ) -> bool;
    /// CPU T-cycles per PPU dot: 1 at normal speed, 2 under CGB double speed.
    fn steps_per_dot(&self) -> u8;
    fn step_instr(&mut self) -> StepResult;
    fn take_instruction_boundary(&mut self);
    /// Settle a STOP the CPU has landed on (arm the CGB speed-switch blackout)
    /// and engage/release a VRAM-DMA CPU hold — `step()` does both at each
    /// instruction boundary; a tcycle driver must call them there too or a
    /// KEY1 switch never re-engages (blank screen) and GDMA/HDMA never runs.
    fn resolve_stop(&mut self, tcycles: u32);
    fn manage_dma_hold(&mut self);
    /// True while a CGB double-speed switch holds the CPU in the settling
    /// blackout. `execute_tcycle_observed` can't advance the blackout — only
    /// `step()`/`step_blackout_chunk` drains it — so the run loop falls back to
    /// `step_instr` while this holds.
    fn speed_switch_in_progress(&self) -> bool;
    /// RGB555 (each channel 0-31) at a screen coordinate, for screenshot
    /// reference matching at the CGB's native colour precision.
    fn rgb555_at(&self, x: usize, y: usize) -> [u8; 3];
    /// 2-bit shade (0=lightest..3=darkest) at a screen coordinate, for snapshotting
    /// the full framebuffer into the pix field at a fixed cycle budget.
    fn shade_at(&self, x: usize, y: usize) -> u8;
    fn drain_audio(&mut self) -> Vec<(f32, f32)>;
}

impl Console for GameBoy {
    fn observe_tcycle(
        &mut self,
        after: impl FnMut(&mut Self, &PhaseResult) -> ControlFlow<()>,
    ) -> bool {
        GameBoy::execute_tcycle_observed(self, after)
    }
    fn steps_per_dot(&self) -> u8 {
        GameBoy::cpu_steps_per_dot(self)
    }
    fn step_instr(&mut self) -> StepResult {
        GameBoy::step(self)
    }
    fn take_instruction_boundary(&mut self) {
        self.cpu_mut().take_instruction_boundary();
    }
    fn resolve_stop(&mut self, tcycles: u32) {
        GameBoy::resolve_stop(self, tcycles);
    }
    fn manage_dma_hold(&mut self) {
        GameBoy::manage_dma_hold(self);
    }
    fn speed_switch_in_progress(&self) -> bool {
        GameBoy::speed_switch_in_progress(self)
    }
    fn rgb555_at(&self, x: usize, y: usize) -> [u8; 3] {
        // DMG screen stores a 2-bit shade index → greyscale RGB555.
        let v = GREY555[self.screen().front().pixels[y][x].0 as usize];
        [v, v, v]
    }
    fn shade_at(&self, x: usize, y: usize) -> u8 {
        self.screen().front().pixels[y][x].0
    }
    fn drain_audio(&mut self) -> Vec<(f32, f32)> {
        GameBoy::drain_audio_samples(self)
    }
}

impl Console for GameBoyColor {
    fn observe_tcycle(
        &mut self,
        after: impl FnMut(&mut Self, &PhaseResult) -> ControlFlow<()>,
    ) -> bool {
        GameBoyColor::execute_tcycle_observed(self, after)
    }
    fn steps_per_dot(&self) -> u8 {
        GameBoyColor::cpu_steps_per_dot(self)
    }
    fn step_instr(&mut self) -> StepResult {
        GameBoyColor::step(self)
    }
    fn take_instruction_boundary(&mut self) {
        self.cpu_mut().take_instruction_boundary();
    }
    fn resolve_stop(&mut self, tcycles: u32) {
        GameBoyColor::resolve_stop(self, tcycles);
    }
    fn manage_dma_hold(&mut self) {
        GameBoyColor::manage_dma_hold(self);
    }
    fn speed_switch_in_progress(&self) -> bool {
        GameBoyColor::speed_switch_in_progress(self)
    }
    fn rgb555_at(&self, x: usize, y: usize) -> [u8; 3] {
        // CGB screen stores a packed RGB555 value → unpack to one byte per 5-bit channel.
        let p = self.screen().pixel(x as u8, y as u8).0;
        [(p & 0x1F) as u8, ((p >> 5) & 0x1F) as u8, ((p >> 10) & 0x1F) as u8]
    }
    fn shade_at(&self, x: usize, y: usize) -> u8 {
        // CGB has no native 2-bit shade; map displayed luminance to the nearest
        // greyscale shade (GREY555 sums 93/63/30/0) so dark-on-light text snapshots
        // faithfully for the gambatte hex check.
        let [r, g, b] = self.rgb555_at(x, y);
        match r as u16 + g as u16 + b as u16 {
            0..=15 => 3,
            16..=46 => 2,
            47..=77 => 1,
            _ => 0,
        }
    }
    fn drain_audio(&mut self) -> Vec<(f32, f32)> {
        GameBoyColor::drain_audio_samples(self)
    }
}

/// T-cycles in one DMG/CGB frame. Used as a time-based safety budget so a
/// ROM that disables the LCD (no frame is ever produced) still terminates
/// instead of spinning the frame-count loop forever — gbmicrotest toggle_lcdc
/// is the canonical offender. missingno's own test harness notes the same
/// trap and bounds by step count for the same reason.
const CYCLES_PER_FRAME: u64 = 70224;

fn framebuffer_to_rgb555<C: Console>(gb: &C) -> Vec<u8> {
    let mut buf = Vec::with_capacity(160 * 144 * 3);
    for y in 0..144 {
        for x in 0..160 {
            buf.extend_from_slice(&gb.rgb555_at(x, y));
        }
    }
    buf
}

/// Last-frame audio-activity check, matching gambatte's testrunner
/// convention (the final frame's samples either all match its first
/// sample → silent, or differ → audio). Tolerance accounts for APU
/// DC-offset drift.
fn last_frame_has_audio(samples: &[(f32, f32)], frames: u32) -> bool {
    if samples.is_empty() || frames == 0 {
        return false;
    }
    let per_frame = (samples.len() / frames as usize).max(1);
    let last = &samples[samples.len().saturating_sub(per_frame)..];
    let (l0, r0) = last[0];
    last.iter()
        .any(|&(l, r)| (l - l0).abs() > 0.005 || (r - r0).abs() > 0.005)
}

fn main() {
    let args = Args::parse();

    let rom_data = fs::read(&args.rom).unwrap_or_else(|e| {
        eprintln!("Error: failed to read ROM {}: {e}", args.rom.display());
        process::exit(1);
    });

    let cartridge = Cartridge::new(rom_data, None);

    let is_cgb = matches!(args.model.to_ascii_lowercase().as_str(), "cgb" | "gbc");
    if is_cgb {
        // missingno-gbc targets CPU-CGB-C (gambatte's cgb04c) — same model
        // gambatte's adapter reports, so cross-emulator CGB diffs line up.
        run(GameBoyColor::new(cartridge, None), &args, "CGB-C");
    } else {
        run(GameBoy::new(cartridge, None), &args, "DMG-B");
    }
}

fn run<C: Console>(mut gb: C, args: &Args, model: &str) {
    let profile = Profile::load(&args.profile).unwrap_or_else(|e| {
        eprintln!("Error: failed to load profile {}: {e}", args.profile.display());
        process::exit(1);
    });

    let mut tracer = Tracer::create(&args.output, &profile, &gb, BootRom::Skip, model).unwrap_or_else(|e| {
        eprintln!("Error: failed to create tracer: {e}");
        process::exit(1);
    });

    // Mark entry 0 as a frame boundary so the setup period is included.
    tracer.mark_frame().unwrap();

    // Discard any startup audio so `--report-audio` measures only the run.
    if args.report_audio {
        let _ = gb.drain_audio();
    }

    let reference_pix = args.reference.as_ref().map(load_reference);
    let is_tcycle = tracer.trigger() == Trigger::Tcycle;
    // CGB/AGB output is colour → push RGB555 pixels (matching the header's
    // pix_format set by Tracer::create); DMG pushes 2-bit shades.
    let cgb = model.starts_with("CGB") || model.starts_with("AGB");

    let mut frame_count: u32 = 0;
    let mut stop_triggered = false;
    let mut remaining_extra: Option<u32> = None;
    let mut serial_match_count: u32 = 0;

    // Detect serial writes by watching SC bit 7 (transfer start)
    let mut prev_sc_high = (gb.peek(0xFF02) & 0x80) != 0;

    // Time-based safety budget: bounds the run even when the LCD never turns on
    // and `frame_count` can't advance. One frame of slack keeps it from ever
    // truncating a legitimate `--frames`-bounded run.
    let max_tcycles = (args.frames as u64)
        .saturating_add(1)
        .saturating_mul(CYCLES_PER_FRAME);
    let mut total_tcycles: u64 = 0;

    // Cycle-budget mode (gambatte hex/blank tests): the harness passes a budget
    // of N × 70224 dots. Sample after N real *frames* (vblanks), not N CPU
    // T-cycles — matching missingno's own `run_frames(N)`. A raw CPU-T-cycle
    // budget under-runs CGB double speed (a real frame is 2× the T-cycles), so
    // the `_ds_` result isn't on screen yet at the budget. At single speed the
    // two are identical (1 dot = 1 T-cycle).
    let sample_frames = args.until_tcycle.map(|b| (b / CYCLES_PER_FRAME) as u32);

    loop {
        // Cycle-budget mode: stop after the derived number of real frames and
        // snapshot the screen at that instant (see `sample_frames` above).
        if let Some(sf) = sample_frames {
            if frame_count >= sf {
                eprintln!("Frame budget reached ({frame_count} frames, {total_tcycles} cycles)");
                break;
            }
        }
        if frame_count >= args.frames {
            eprintln!("Frame limit reached ({} frames)", args.frames);
            break;
        }

        if total_tcycles >= max_tcycles {
            eprintln!("T-cycle limit reached ({total_tcycles} cycles; LCD likely off)");
            break;
        }

        if let Some(ref mut remaining) = remaining_extra {
            if *remaining == 0 {
                break;
            }
        }

        let (new_screen, tcycles) = if is_tcycle && !gb.speed_switch_in_progress() {
            step_tcycle(&mut gb, &mut tracer, cgb)
        } else {
            // During a CGB speed-switch blackout the tcycle driver can't advance
            // the frozen CPU (`execute_tcycle_observed` never re-engages it);
            // only `step()`/`step_blackout_chunk` drains it. Fall back to
            // instruction stepping for the blackout, then resume tcycle capture.
            step_instruction(&mut gb, &mut tracer)
        };
        total_tcycles += tcycles;

        if !stop_triggered {
            if let Some(opcode) = args.stop_opcode {
                let pc = gb.cpu().pc;
                if gb.peek(pc) == opcode {
                    eprintln!("Stop condition met: opcode 0x{opcode:02X} at PC=0x{pc:04X}");
                    stop_triggered = true;
                    remaining_extra = Some(args.extra_frames);
                }
            }

            for sw in &args.stop_when {
                let actual = gb.peek(sw.addr);
                let hit = if sw.negate { actual != sw.value } else { actual == sw.value };
                if hit {
                    let op = if sw.negate { "!=" } else { "==" };
                    eprintln!("Stop condition met: [0x{:04X}] {op} 0x{:02X}", sw.addr, sw.value);
                    stop_triggered = true;
                    remaining_extra = Some(args.extra_frames);
                    break;
                }
            }

            if let Some(serial_byte) = args.stop_on_serial {
                let sc_high = (gb.peek(0xFF02) & 0x80) != 0;
                if sc_high && !prev_sc_high {
                    let sb = gb.peek(0xFF01);
                    if sb == serial_byte {
                        serial_match_count += 1;
                        if serial_match_count >= args.stop_serial_count {
                            eprintln!(
                                "Stop condition met: serial byte 0x{serial_byte:02X} (count {serial_match_count})"
                            );
                            stop_triggered = true;
                            remaining_extra = Some(args.extra_frames);
                        }
                    }
                }
                prev_sc_high = sc_high;
            }
        }

        // Reference screenshot check runs on every frame boundary, even
        // after other stop conditions fire (the screen may not have
        // updated yet when serial/opcode triggers).
        if new_screen {
            if let Some(ref reference) = reference_pix {
                let current = framebuffer_to_rgb555(&gb);
                if rgb555_match(&current, reference) {
                    if !stop_triggered {
                        stop_triggered = true;
                        remaining_extra = Some(args.extra_frames);
                    }
                    eprintln!("Reference match at frame {}", frame_count + 1);
                }
            }
        }

        if new_screen {
            frame_count += 1;
            if let Some(ref mut remaining) = remaining_extra {
                *remaining = remaining.saturating_sub(1);
            }
        }
    }

    // Cycle-budget mode: emit the full framebuffer at the budget as the trace's
    // final frame. The per-dot pix only holds the partial in-progress frame, so
    // we snapshot the whole screen (which still shows the persistent result) as
    // one frame — this is the screen the gambatte hex/blank check reads.
    if args.until_tcycle.is_some() {
        tracer.mark_frame().unwrap();
        for y in 0..144 {
            for x in 0..160 {
                if cgb {
                    tracer.push_pixel_rgb555(rgb555_u16(&gb, x as u8, y as u8));
                } else {
                    tracer.push_pixel(gb.shade_at(x, y));
                }
            }
        }
        tracer.capture(&gb).unwrap();
    }

    if args.report_audio {
        let samples = gb.drain_audio();
        let has_audio = last_frame_has_audio(&samples, frame_count.max(1));
        eprintln!("AUDIO={}", if has_audio { 1 } else { 0 });
    }

    tracer.finish().unwrap_or_else(|e| {
        eprintln!("Error finalizing trace: {e}");
        process::exit(1);
    });

    eprintln!("Trace written: {frame_count} frames");
}

/// RGB555 (15-bit) value of the screen pixel at (x, y), packed for the pix field.
fn rgb555_u16<C: Console>(gb: &C, x: u8, y: u8) -> u16 {
    let [r, g, b] = gb.rgb555_at(x as usize, y as usize); // each channel 0-31
    ((r as u16) << 10) | ((g as u16) << 5) | (b as u16)
}

/// Step one instruction via T-cycle phases, capturing at each dot.
/// Returns `(new_screen, tcycles_captured)`. `cgb` selects the pix encoding:
/// RGB555 colour (CGB) vs 2-bit shade (DMG).
///
/// Mirrors missingno's own `trace::step_instruction_tcycle`: at single speed we
/// capture once per T-cycle (after the fall, OR-ing both edges' frame flag); at
/// CGB double speed the CPU runs at 2× the dot clock, so we capture after every
/// master edge and may retire mid-pair — the `Break` defers the unpaired fall to
/// the next `observe_tcycle` call.
fn step_tcycle<C: Console>(gb: &mut C, tracer: &mut Tracer, cgb: bool) -> (bool, u64) {
    let mut new_screen = false;
    let mut tcycles: u64 = 0;

    gb.take_instruction_boundary();
    let double_speed = gb.steps_per_dot() == 2;

    loop {
        let mut first_new_screen = false;
        let mut is_first = true;
        gb.observe_tcycle(|gb, result| {
            new_screen |= result.new_screen;
            if let Some(pixel) = result.pixel {
                if cgb {
                    tracer.push_pixel_rgb555(rgb555_u16(&*gb, pixel.x, pixel.y));
                } else {
                    tracer.push_pixel(pixel.shade);
                }
            }
            if double_speed {
                // Capture after every edge; the pair may retire on the rise.
                if result.new_screen {
                    tracer.mark_frame().unwrap();
                }
                tracer.capture(&*gb).unwrap();
                tracer.advance_dot();
                tcycles += 1;
                if gb.cpu().at_instruction_boundary() {
                    return ControlFlow::Break(());
                }
            } else if is_first {
                // Single speed: defer capture to the fall, OR-ing the rise's flag.
                first_new_screen = result.new_screen;
                is_first = false;
            } else {
                if first_new_screen || result.new_screen {
                    tracer.mark_frame().unwrap();
                }
                tracer.capture(&*gb).unwrap();
                tracer.advance_dot();
                tcycles += 1;
            }
            ControlFlow::Continue(())
        });

        if gb.cpu().at_instruction_boundary() {
            break;
        }
    }

    // Mirror `step`/missingno's `trace::step_instruction_tcycle`: settle a
    // landed STOP (arm the CGB speed-switch blackout) and engage/release a
    // VRAM-DMA CPU hold at the boundary, so traced runs progress past STOP and
    // run their GDMA/HDMA like untraced ones. Without this, `_ds_` tests never
    // re-engage (blank screen) and `dma__*` transfers never run.
    gb.resolve_stop(tcycles as u32);
    gb.manage_dma_hold();

    (new_screen, tcycles)
}

/// Step one instruction, capture once.
/// Returns `(new_screen, tcycles_consumed)`.
fn step_instruction<C: Console>(gb: &mut C, tracer: &mut Tracer) -> (bool, u64) {
    tracer.capture(&*gb).unwrap();
    let result = gb.step_instr();
    tracer.advance(result.tcycles);

    if result.new_screen {
        tracer.mark_frame().unwrap();
    }

    (result.new_screen, result.tcycles as u64)
}
