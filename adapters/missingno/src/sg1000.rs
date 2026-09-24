//! morepork-missingno-sg1000 — drive missingno's SG-1000 under the TI VDP
//! corpus adapter contract:
//!
//!     morepork-missingno-sg1000 -rom test.sg -out trace.morepork [-frames N] [-spec PAL]
//!
//! One entry per instruction through the core's own capture bridge (its
//! state schema's columns plus the shared trace observations). Tracing stops
//! when RESULT ($C000) holds a verdict, or after `-frames` frames; the run
//! then continues to the next completed frame, which is embedded.

use std::process::ExitCode;

use missingno_sg1000::console::{tstates_per_frame, Sg1000};
use missingno_sg1000::trace::{TraceScope, Tracer, Trigger};

#[path = "ti_vdp_args.rs"]
mod ti_vdp_args;

const RESULT: u16 = 0xC000;
const USAGE: &str =
    "usage: morepork-missingno-sg1000 -rom <file.sg> [-out <trace>] [-frames <n>] [-spec NTSC|PAL]";

fn run(args: &ti_vdp_args::Args) -> Result<(), String> {
    let rom = std::fs::read(&args.rom).map_err(|e| format!("{}: {e}", args.rom))?;
    let mut sg = Sg1000::new(&rom, None, args.standard).map_err(|e| format!("cartridge: {e:?}"))?;
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
        tracer.capture(&mut sg).map_err(|e| e.to_string())?;
        if ti_vdp_args::is_verdict(sg.peek(RESULT)) {
            verdict = true;
            break;
        }
        if frames_done >= args.frames {
            break;
        }
        sg.step_instruction();
        if sg.take_frame().is_some() {
            frames_done += 1;
        }
    }

    let frame = sg.step_frame(2 * tstates_per_frame(args.standard));
    tracer.mark_frame(frame).map_err(|e| e.to_string())?;
    tracer.finish().map_err(|e| e.to_string())?;
    eprintln!("trace written: {frames_done} frames, verdict={verdict}");
    Ok(())
}

fn main() -> ExitCode {
    match ti_vdp_args::parse(USAGE, false).and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("morepork-missingno-sg1000: {err}");
            ExitCode::FAILURE
        }
    }
}
