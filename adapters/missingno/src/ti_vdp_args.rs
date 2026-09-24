//! The TI VDP corpus adapter contract shared by the SG-1000 and ColecoVision
//! drivers: `-rom <file> [-out trace.morepork] [-frames N] [-spec NTSC|PAL]`.

use missingno_ti_vdp::Standard;

pub struct Args {
    pub rom: String,
    pub out: String,
    pub frames: u32,
    pub standard: Standard,
    pub bios: Option<String>,
}

/// Parse the contract's flags; `-bios` is accepted only where `takes_bios`.
pub fn parse(usage: &str, takes_bios: bool) -> Result<Args, String> {
    let mut args = Args {
        rom: String::new(),
        out: "trace.morepork".into(),
        frames: 600,
        standard: Standard::Ntsc,
        bios: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-rom" => args.rom = value("-rom")?,
            "-out" => args.out = value("-out")?,
            "-frames" => {
                args.frames = value("-frames")?
                    .parse()
                    .map_err(|e| format!("-frames: {e}"))?
            }
            "-spec" => {
                args.standard = match value("-spec")?.to_uppercase().as_str() {
                    "NTSC" => Standard::Ntsc,
                    "PAL" => Standard::Pal,
                    other => return Err(format!("unknown spec '{other}' (NTSC, PAL)")),
                }
            }
            "-bios" if takes_bios => args.bios = Some(value("-bios")?),
            other => return Err(format!("unknown flag '{other}'\n{usage}")),
        }
    }
    if args.rom.is_empty() {
        return Err(usage.into());
    }
    Ok(args)
}

/// Whether the corpus RESULT byte holds a terminal verdict ($A5 PASS / $5A FAIL).
pub fn is_verdict(result: u8) -> bool {
    result == 0xA5 || result == 0x5A
}
