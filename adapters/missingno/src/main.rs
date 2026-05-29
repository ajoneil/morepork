use std::fs;
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
trait Console: Traceable {
    fn step_phase(&mut self) -> PhaseResult;
    fn step_instr(&mut self) -> StepResult;
    fn take_instruction_boundary(&mut self);
    /// RGB555 (each channel 0-31) at a screen coordinate, for screenshot
    /// reference matching at the CGB's native colour precision.
    fn rgb555_at(&self, x: usize, y: usize) -> [u8; 3];
    fn drain_audio(&mut self) -> Vec<(f32, f32)>;
}

impl Console for GameBoy {
    fn step_phase(&mut self) -> PhaseResult {
        GameBoy::step_phase(self)
    }
    fn step_instr(&mut self) -> StepResult {
        GameBoy::step(self)
    }
    fn take_instruction_boundary(&mut self) {
        self.cpu_mut().take_instruction_boundary();
    }
    fn rgb555_at(&self, x: usize, y: usize) -> [u8; 3] {
        // DMG screen stores a 2-bit shade index → greyscale RGB555.
        let v = GREY555[self.screen().front().pixels[y][x].0 as usize];
        [v, v, v]
    }
    fn drain_audio(&mut self) -> Vec<(f32, f32)> {
        GameBoy::drain_audio_samples(self)
    }
}

impl Console for GameBoyColor {
    fn step_phase(&mut self) -> PhaseResult {
        GameBoyColor::step_phase(self)
    }
    fn step_instr(&mut self) -> StepResult {
        GameBoyColor::step(self)
    }
    fn take_instruction_boundary(&mut self) {
        self.cpu_mut().take_instruction_boundary();
    }
    fn rgb555_at(&self, x: usize, y: usize) -> [u8; 3] {
        // CGB screen stores RGB888 → reduce to native 5-bit precision.
        let p = self.screen().pixel(x as u8, y as u8);
        [p.r >> 3, p.g >> 3, p.b >> 3]
    }
    fn drain_audio(&mut self) -> Vec<(f32, f32)> {
        GameBoyColor::drain_audio_samples(self)
    }
}

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

    let mut frame_count: u32 = 0;
    let mut stop_triggered = false;
    let mut remaining_extra: Option<u32> = None;
    let mut serial_match_count: u32 = 0;

    // Detect serial writes by watching SC bit 7 (transfer start)
    let mut prev_sc_high = (gb.peek(0xFF02) & 0x80) != 0;

    loop {
        if frame_count >= args.frames {
            eprintln!("Frame limit reached ({} frames)", args.frames);
            break;
        }

        if let Some(ref mut remaining) = remaining_extra {
            if *remaining == 0 {
                break;
            }
        }

        let new_screen = if is_tcycle {
            step_tcycle(&mut gb, &mut tracer)
        } else {
            step_instruction(&mut gb, &mut tracer)
        };

        if !stop_triggered {
            if let Some(opcode) = args.stop_opcode {
                let pc = gb.cpu().bus_counter;
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

/// Step one instruction via T-cycle phases, capturing at each dot.
fn step_tcycle<C: Console>(gb: &mut C, tracer: &mut Tracer) -> bool {
    let mut new_screen = false;

    gb.take_instruction_boundary();

    loop {
        let rise = gb.step_phase();
        new_screen |= rise.new_screen;
        if let Some(pixel) = rise.pixel {
            tracer.push_pixel(pixel.shade);
        }

        let fall = gb.step_phase();
        new_screen |= fall.new_screen;
        if let Some(pixel) = fall.pixel {
            tracer.push_pixel(pixel.shade);
        }

        if rise.new_screen || fall.new_screen {
            tracer.mark_frame().unwrap();
        }

        tracer.capture(&*gb).unwrap();
        tracer.advance_dot();

        if gb.cpu().at_instruction_boundary() {
            break;
        }
    }

    new_screen
}

/// Step one instruction, capture once.
fn step_instruction<C: Console>(gb: &mut C, tracer: &mut Tracer) -> bool {
    tracer.capture(&*gb).unwrap();
    let result = gb.step_instr();
    tracer.advance(result.tcycles);

    if result.new_screen {
        tracer.mark_frame().unwrap();
    }

    result.new_screen
}
