//! The Game Boy field catalogue: every field an adapter can capture, with
//! type, nullability, and encoding fixed per subsystem layer. This is the
//! family's default catalogue for profile validation and the permanent
//! type-resolution fallback for legacy traces whose headers predate
//! `field_defs`.

use crate::profile::{FieldDef, FieldType, Layer, SubsystemDef};

use crate::family::field;



pub static CPU: SubsystemDef = SubsystemDef {
    name: "cpu",
    layers: &[
        (Layer::Registers, &[
            field!("pc", u16),
            field!("op_addr", u16),
            field!("sp", u16),
            field!("a", u8),
            field!("f", u8, dict),
            field!("b", u8),
            field!("c", u8),
            field!("d", u8),
            field!("e", u8),
            field!("h", u8),
            field!("l", u8),
            field!("ime", bool),
            field!("op_state", u8),
            field!("mcycle_phase", u8),
            field!("halted", bool),
        ]),
        (Layer::Internal, &[
            field!("bus_addr", u16),
        ]),
        (Layer::Timing, &[
            field!("mcycles", u8),
            field!("tcycles", u8),
        ]),
    ],
};

pub static PPU: SubsystemDef = SubsystemDef {
    name: "ppu",
    layers: &[
        (Layer::Registers, &[
            field!("lcdc", u8, dict),
            field!("stat", u8, dict),
            field!("ly", u8),
            field!("lyc", u8),
            field!("scy", u8),
            field!("scx", u8),
            field!("wy", u8),
            field!("wx", u8),
            field!("bgp", u8, dict),
            field!("obp0", u8, dict),
            field!("obp1", u8, dict),
            field!("dma", u8),
        ]),
        (Layer::Internal, &[
            // sprite store (10 sprites × 3 fields)
            field!("oam0_x", u8), field!("oam0_id", u8), field!("oam0_attr", u8),
            field!("oam1_x", u8), field!("oam1_id", u8), field!("oam1_attr", u8),
            field!("oam2_x", u8), field!("oam2_id", u8), field!("oam2_attr", u8),
            field!("oam3_x", u8), field!("oam3_id", u8), field!("oam3_attr", u8),
            field!("oam4_x", u8), field!("oam4_id", u8), field!("oam4_attr", u8),
            field!("oam5_x", u8), field!("oam5_id", u8), field!("oam5_attr", u8),
            field!("oam6_x", u8), field!("oam6_id", u8), field!("oam6_attr", u8),
            field!("oam7_x", u8), field!("oam7_id", u8), field!("oam7_attr", u8),
            field!("oam8_x", u8), field!("oam8_id", u8), field!("oam8_attr", u8),
            field!("oam9_x", u8), field!("oam9_id", u8), field!("oam9_attr", u8),
            // pixel FIFO
            field!("bgw_fifo_a", u8), field!("bgw_fifo_b", u8),
            field!("spr_fifo_a", u8), field!("spr_fifo_b", u8),
            field!("mask_pipe", u8), field!("pal_pipe", u8),
            // fetcher
            field!("tfetch_state", u8, dict), field!("sfetch_state", u8, dict),
            field!("tile_temp_a", u8), field!("tile_temp_b", u8),
            // counters/flags
            field!("pix_count", u8), field!("sprite_count", u8), field!("scan_count", u8),
            field!("rendering", bool), field!("win_mode", bool),
        ]),
        (Layer::Writes, &[
            field!("vram_addr", u16, nullable),
            field!("vram_data", u8, nullable),
        ]),
        (Layer::Output, &[
            field!("pix", str, nullable),
            field!("pix_x", u8),
        ]),
    ],
};

pub static APU: SubsystemDef = SubsystemDef {
    name: "apu",
    layers: &[
        (Layer::Registers, &[
            // Channel 1 — square with sweep
            field!("ch1_sweep", u8), field!("ch1_duty_len", u8), field!("ch1_vol_env", u8),
            field!("ch1_freq_lo", u8), field!("ch1_freq_hi", u8),
            // Channel 2 — square
            field!("ch2_duty_len", u8), field!("ch2_vol_env", u8),
            field!("ch2_freq_lo", u8), field!("ch2_freq_hi", u8),
            // Channel 3 — wave
            field!("ch3_dac", u8), field!("ch3_len", u8), field!("ch3_vol", u8),
            field!("ch3_freq_lo", u8), field!("ch3_freq_hi", u8),
            // Channel 4 — noise
            field!("ch4_len", u8), field!("ch4_vol_env", u8),
            field!("ch4_freq", u8), field!("ch4_control", u8),
            // Control
            field!("master_vol", u8), field!("sound_pan", u8), field!("sound_on", u8),
        ]),
        (Layer::Internal, &[
            // Channel 1 — square with sweep
            field!("ch1_active", bool),
            field!("ch1_freq_cnt", u16),
            field!("ch1_env_vol", u8),
            field!("ch1_phase", u8),
            field!("ch1_sweep_shadow", u16),
            field!("ch1_len_cnt", u8),
            // Channel 2 — square
            field!("ch2_active", bool),
            field!("ch2_freq_cnt", u16),
            field!("ch2_env_vol", u8),
            field!("ch2_phase", u8),
            field!("ch2_len_cnt", u8),
            // Channel 3 — wave
            field!("ch3_active", bool),
            field!("ch3_freq_cnt", u16),
            field!("ch3_wave_idx", u8),
            field!("ch3_sample", u8),
            field!("ch3_len_cnt", u8),
            // Channel 4 — noise
            field!("ch4_active", bool),
            field!("ch4_freq_cnt", u16),
            field!("ch4_env_vol", u8),
            field!("ch4_lfsr", u16),
            field!("ch4_len_cnt", u8),
        ]),
        (Layer::Writes, &[
            field!("apu_write_addr", u16, nullable),
            field!("apu_write_data", u8, nullable),
        ]),
    ],
};

pub static TIMER: SubsystemDef = SubsystemDef {
    name: "timer",
    layers: &[
        (Layer::Registers, &[
            field!("div", u8),
            field!("tima", u8),
            field!("tma", u8),
            field!("tac", u8, dict),
        ]),
    ],
};

pub static INTERRUPT: SubsystemDef = SubsystemDef {
    name: "interrupt",
    layers: &[
        (Layer::Registers, &[
            field!("if_", u8),
            field!("ie", u8),
        ]),
        // CPU interrupt-dispatch DFFs from PPU spec §13.2. Names are the
        // spec's semantic handles. `dispatch_trigger` (combinational
        // pulse) and `ime_pending` (EI delay SR latch) are deferred —
        // their value is sub-M-cycle and adapters' modeling differs.
        (Layer::Internal, &[
            field!("irq_pending", bool),
            field!("dispatch_active", bool),
            field!("irq_latched", bool),
        ]),
    ],
};

pub static SERIAL: SubsystemDef = SubsystemDef {
    name: "serial",
    layers: &[
        (Layer::Registers, &[
            field!("sb", u8),
            field!("sc", u8),
        ]),
    ],
};

/// All subsystems in field order.
pub static SUBSYSTEMS: &[&SubsystemDef] = &[
    &CPU, &PPU, &APU, &TIMER, &INTERRUPT, &SERIAL,
];

