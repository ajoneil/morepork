//! morepork-missingno-colecovision — drive missingno's ColecoVision under the
//! TI VDP corpus adapter contract:
//!
//!     morepork-missingno-colecovision -rom test.col -out trace.morepork \
//!         [-frames N] [-spec PAL] [-bios <file>]
//!
//! The BIOS image is not bundled: `-bios`, or the `COLECOVISION_BIOS`
//! environment variable. One entry per instruction through the core's own
//! capture bridge. Tracing stops when RESULT ($7000) holds a verdict, or
//! after `-frames` frames; the run then continues to the next completed
//! frame, which is embedded.

use std::process::ExitCode;

use missingno_colecovision::console::{tstates_per_frame, ColecoVision};
use missingno_colecovision::firmware::BIOS_SIZE;
use missingno_colecovision::trace::{TraceScope, Tracer, Trigger};

#[path = "ti_vdp_args.rs"]
mod ti_vdp_args;

const RESULT: u16 = 0x7000;
const USAGE: &str = "usage: morepork-missingno-colecovision -rom <file.col> [-out <trace>] [-frames <n>] [-spec NTSC|PAL] [-bios <file>] (or COLECOVISION_BIOS)";

fn run(args: &ti_vdp_args::Args) -> Result<(), String> {
    let rom = std::fs::read(&args.rom).map_err(|e| format!("{}: {e}", args.rom))?;
    let bios_path = args
        .bios
        .clone()
        .or_else(|| std::env::var("COLECOVISION_BIOS").ok())
        .ok_or("no BIOS: pass -bios or set COLECOVISION_BIOS")?;
    let bios: [u8; BIOS_SIZE] = std::fs::read(&bios_path)
        .map_err(|e| format!("{bios_path}: {e}"))?
        .try_into()
        .map_err(|image: Vec<u8>| format!("{bios_path}: {} bytes, not {BIOS_SIZE}", image.len()))?;
    let mut cv =
        ColecoVision::new(&rom, args.standard, bios).map_err(|e| format!("cartridge: {e:?}"))?;
    let mut tracer = Tracer::create(
        &args.out,
        &rom,
        args.standard,
        Trigger::Instruction,
        TraceScope::Full,
    )
    .map_err(|e| e.to_string())?;

    let mut frames_done = 0;
    let mut verdict = false;
    loop {
        tracer.capture(&mut cv).map_err(|e| e.to_string())?;
        if ti_vdp_args::is_verdict(cv.peek(RESULT)) {
            verdict = true;
            break;
        }
        if frames_done >= args.frames {
            break;
        }
        cv.step_instruction();
        if cv.take_frame().is_some() {
            frames_done += 1;
        }
    }

    let frame = cv.step_frame(2 * tstates_per_frame(args.standard));
    tracer.mark_frame(frame).map_err(|e| e.to_string())?;
    tracer.finish().map_err(|e| e.to_string())?;
    eprintln!("trace written: {frames_done} frames, verdict={verdict}");
    Ok(())
}

fn main() -> ExitCode {
    match ti_vdp_args::parse(USAGE, true).and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("morepork-missingno-colecovision: {err}");
            ExitCode::FAILURE
        }
    }
}
