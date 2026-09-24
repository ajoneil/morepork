//! The registry types every adapter's columns, expands profiles over each
//! system's vocabulary, and completes producers' headers.

use std::collections::BTreeMap;

use morepork::header::{ExtensionField, TraceHeader};
use morepork::profile::FieldType;
use morepork_systems::{column_defs, describe, parse_profile, Error};

const Z80_TI_VDP: &[&str] = &[
    "pc",
    "sp",
    "a",
    "f",
    "b",
    "c",
    "d",
    "e",
    "h",
    "l",
    "ix",
    "iy",
    "wz",
    "a_alt",
    "f_alt",
    "b_alt",
    "c_alt",
    "d_alt",
    "e_alt",
    "h_alt",
    "l_alt",
    "i",
    "r",
    "im",
    "iff1",
    "iff2",
    "halted",
    "vdp_r0",
    "vdp_r1",
    "vdp_r2",
    "vdp_r3",
    "vdp_r4",
    "vdp_r5",
    "vdp_r6",
    "vdp_r7",
    "vdp_frame_flag",
    "vdp_fifth_sprite_flag",
    "vdp_coincidence_flag",
    "vdp_fifth_sprite_index",
    "vdp_address",
    "vdp_awaiting_second_byte",
    "vdp_read_buffer",
    "vdp_line",
    "vdp_line_xtal",
    "result",
    "code",
    "observed",
    "expected",
];

const OPENMSX: &[&str] = &[
    "pc",
    "sp",
    "a",
    "f",
    "b",
    "c",
    "d",
    "e",
    "h",
    "l",
    "ix",
    "iy",
    "vdp_r0",
    "vdp_r1",
    "vdp_r2",
    "vdp_r3",
    "vdp_r4",
    "vdp_r5",
    "vdp_r6",
    "vdp_r7",
    "vdp_frame_flag",
    "vdp_fifth_sprite_flag",
    "vdp_coincidence_flag",
    "vdp_fifth_sprite_index",
    "vdp_address",
    "vdp_awaiting_second_byte",
    "vdp_read_buffer",
    "result",
    "code",
    "observed",
    "expected",
];

const MAME_Z80: &[&str] = &[
    "pc", "sp", "a", "f", "b", "c", "d", "e", "h", "l", "ix", "iy", "result", "code", "observed",
    "expected",
];

const MAME_VCS: &[&str] = &[
    "pc", "a", "x", "y", "s", "p", "result", "code", "observed", "expected",
];

const STELLA: &[&str] = &[
    "pc", "a", "x", "y", "s", "p", "line", "beam", "result", "code", "observed", "expected",
];

const GOPHER2600: &[&str] = &[
    "pc",
    "a",
    "x",
    "y",
    "s",
    "p",
    "line",
    "beam",
    "riot_timer",
    "riot_porta_pins",
    "riot_portb_pins",
    "result",
    "code",
    "observed",
    "expected",
];

const MISSINGNO_VCS: &[&str] = &[
    "pc",
    "a",
    "x",
    "y",
    "s",
    "p",
    "cpu_ready",
    "cycles",
    "line",
    "beam",
    "riot_timer",
    "riot_porta_pins",
    "riot_portb_pins",
    "result",
    "code",
    "observed",
    "expected",
];

/// The IO-register map the C Game Boy adapters share.
const GB_IO: &[&str] = &[
    "lcdc",
    "stat",
    "scy",
    "scx",
    "ly",
    "lyc",
    "wy",
    "wx",
    "bgp",
    "obp0",
    "obp1",
    "dma",
    "div",
    "tima",
    "tma",
    "tac",
    "if_",
    "ie",
    "sb",
    "sc",
    "ch1_sweep",
    "ch1_duty_len",
    "ch1_vol_env",
    "ch1_freq_lo",
    "ch1_freq_hi",
    "ch2_duty_len",
    "ch2_vol_env",
    "ch2_freq_lo",
    "ch2_freq_hi",
    "ch3_dac",
    "ch3_len",
    "ch3_vol",
    "ch3_freq_lo",
    "ch3_freq_hi",
    "ch4_len",
    "ch4_vol_env",
    "ch4_freq",
    "ch4_control",
    "master_vol",
    "sound_pan",
    "sound_on",
];

/// The CPU and pixel columns the C Game Boy adapters can emit (each takes
/// the subset its emulator exposes; see `GB_ADAPTERS`).
const GB_CPU: &[&str] = &[
    "pc", "op_addr", "sp", "a", "f", "b", "c", "d", "e", "h", "l", "ime", "pix",
];

/// Each C Game Boy adapter's columns, as its built binary's header lists them
/// under a profile selecting every subsystem: the system ids it writes, then
/// the `GB_CPU` and `GB_IO` columns it omits. GateBoy adds `GATEBOY_PPU` and
/// its extensions, BGB its extensions.
const GB_ADAPTERS: &[(&str, &[&str], &[&str])] = &[
    ("sameboy", &["dmg", "cgb"], &[]),
    ("gambatte", &["dmg", "cgb"], &["ime"]),
    ("mgba", &["dmg", "cgb"], &["op_addr"]),
    ("docboy", &["dmg", "cgb"], &[]),
    ("gateboy", &["dmg"], &["op_addr", "sb", "sc"]),
    ("bgb", &["dmg"], &["op_addr", "pix"]),
];

/// BGB's gbmicrotest result block, as its header declares it: the HRAM bytes
/// $FF80 (value read), $FF81 (value expected) and $FF82 (verdict).
const BGB_EXTENSIONS: &[&str] = &[
    "gbmicrotest_actual",
    "gbmicrotest_expected",
    "gbmicrotest_result",
];

/// The PPU pipeline cells GateBoy reads beyond the IO map.
const GATEBOY_PPU: &[&str] = &[
    "bgw_fifo_a",
    "bgw_fifo_b",
    "spr_fifo_a",
    "spr_fifo_b",
    "pal_pipe",
    "tfetch_state",
    "sfetch_state",
    "tile_temp_a",
    "tile_temp_b",
    "pix_count",
    "sprite_count",
    "scan_count",
    "rendering",
    "win_mode",
];

/// GateBoy's gate-level columns outside the DMG vocabulary, as its header
/// declares them: name, type, nullable.
const GATEBOY_EXTENSIONS: &[(&str, FieldType, bool)] = &[
    ("bus_addr", FieldType::UInt16, false),
    ("ch1_freq_cnt", FieldType::UInt16, false),
    ("ch1_sweep_shadow", FieldType::UInt16, false),
    ("ch2_freq_cnt", FieldType::UInt16, false),
    ("ch3_freq_cnt", FieldType::UInt16, false),
    ("ch4_freq_cnt", FieldType::UInt16, false),
    ("ch4_lfsr", FieldType::UInt16, false),
    ("op_state", FieldType::UInt8, false),
    ("mcycle_phase", FieldType::UInt8, false),
    ("mask_pipe", FieldType::UInt8, false),
    ("oam0_x", FieldType::UInt8, false),
    ("oam0_id", FieldType::UInt8, false),
    ("oam0_attr", FieldType::UInt8, false),
    ("oam1_x", FieldType::UInt8, false),
    ("oam1_id", FieldType::UInt8, false),
    ("oam1_attr", FieldType::UInt8, false),
    ("oam2_x", FieldType::UInt8, false),
    ("oam2_id", FieldType::UInt8, false),
    ("oam2_attr", FieldType::UInt8, false),
    ("oam3_x", FieldType::UInt8, false),
    ("oam3_id", FieldType::UInt8, false),
    ("oam3_attr", FieldType::UInt8, false),
    ("oam4_x", FieldType::UInt8, false),
    ("oam4_id", FieldType::UInt8, false),
    ("oam4_attr", FieldType::UInt8, false),
    ("oam5_x", FieldType::UInt8, false),
    ("oam5_id", FieldType::UInt8, false),
    ("oam5_attr", FieldType::UInt8, false),
    ("oam6_x", FieldType::UInt8, false),
    ("oam6_id", FieldType::UInt8, false),
    ("oam6_attr", FieldType::UInt8, false),
    ("oam7_x", FieldType::UInt8, false),
    ("oam7_id", FieldType::UInt8, false),
    ("oam7_attr", FieldType::UInt8, false),
    ("oam8_x", FieldType::UInt8, false),
    ("oam8_id", FieldType::UInt8, false),
    ("oam8_attr", FieldType::UInt8, false),
    ("oam9_x", FieldType::UInt8, false),
    ("oam9_id", FieldType::UInt8, false),
    ("oam9_attr", FieldType::UInt8, false),
    ("ch1_env_vol", FieldType::UInt8, false),
    ("ch1_phase", FieldType::UInt8, false),
    ("ch1_len_cnt", FieldType::UInt8, false),
    ("ch2_env_vol", FieldType::UInt8, false),
    ("ch2_phase", FieldType::UInt8, false),
    ("ch2_len_cnt", FieldType::UInt8, false),
    ("ch3_wave_idx", FieldType::UInt8, false),
    ("ch3_sample", FieldType::UInt8, false),
    ("ch3_len_cnt", FieldType::UInt8, false),
    ("ch4_env_vol", FieldType::UInt8, false),
    ("ch4_len_cnt", FieldType::UInt8, false),
    ("ch1_active", FieldType::Bool, false),
    ("ch2_active", FieldType::Bool, false),
    ("ch3_active", FieldType::Bool, false),
    ("ch4_active", FieldType::Bool, false),
    ("halted", FieldType::Bool, false),
    ("irq_pending", FieldType::Bool, false),
    ("dispatch_active", FieldType::Bool, false),
    ("irq_latched", FieldType::Bool, false),
    ("vram_addr", FieldType::UInt16, true),
    ("vram_data", FieldType::UInt8, true),
    ("apu_write_addr", FieldType::UInt16, true),
    ("apu_write_data", FieldType::UInt8, true),
];

fn gateboy_extension_fields() -> BTreeMap<String, ExtensionField> {
    GATEBOY_EXTENSIONS
        .iter()
        .map(|&(name, field_type, nullable)| {
            let ext = ExtensionField {
                field_type,
                nullable,
                description: None,
                source: Some("gateboy".into()),
                subsystem: Some("gateboy".into()),
                layer: Some("internal".into()),
            };
            (name.to_string(), ext)
        })
        .collect()
}

#[test]
fn gateboy_columns_resolve_with_its_extension_declarations() {
    let mut fields: Vec<&str> = GB_IO
        .iter()
        .chain(GB_CPU)
        .chain(GATEBOY_PPU)
        .copied()
        .filter(|n| !["sb", "sc", "op_addr"].contains(n))
        .collect();
    fields.extend(GATEBOY_EXTENSIONS.iter().map(|&(name, _, _)| name));
    assert_eq!(GATEBOY_EXTENSIONS.len(), 63);

    for &(name, _, _) in GATEBOY_EXTENSIONS {
        assert!(
            column_defs("dmg", &[name]).is_err(),
            "{name} is a dmg column"
        );
    }

    let mut h = header("dmg", &fields);
    h.extension_fields = gateboy_extension_fields();
    describe(&mut h).unwrap();
    assert_eq!(h.field_defs.len(), fields.len());
    for &(name, field_type, nullable) in GATEBOY_EXTENSIONS {
        let def = h.field_def(name).unwrap();
        assert_eq!(def.field_type, field_type, "{name}");
        assert_eq!(def.nullable, nullable, "{name}");
        assert_eq!(def.subsystem.as_deref(), Some("gateboy"), "{name}");
        assert_eq!(def.layer.as_deref(), Some("internal"), "{name}");
    }
    assert_eq!(h.field_def("ly").unwrap().subsystem.as_deref(), Some("ppu"));

    let names: Vec<String> = GATEBOY_EXTENSIONS
        .iter()
        .map(|&(name, _, _)| format!("\"{name}\""))
        .collect();
    let p = parse_profile(&format!(
        "[profile]\nname = \"t\"\ndescription = \"t\"\ntrigger = \"tcycle\"\n\n[fields]\nppu = \"registers\"\n\n[fields.extensions]\ngateboy = [{}]\n",
        names.join(", ")
    ))
    .unwrap();
    assert_eq!(p.extensions["gateboy"].len(), 63);
}

#[test]
fn an_extension_field_may_not_shadow_a_vocabulary_name() {
    let mut h = header("dmg", &["pc", "ly"]);
    let mut ext = gateboy_extension_fields();
    let decl = ext.remove("bus_addr").unwrap();
    ext.insert("ly".into(), decl);
    h.extension_fields = ext;
    let err = describe(&mut h).unwrap_err();
    assert!(
        matches!(&err, Error::ShadowedColumn { column, .. } if column == "ly"),
        "{err}"
    );
}

#[test]
fn every_gb_adapter_column_resolves() {
    for system in ["dmg", "cgb"] {
        for columns in [GB_IO, GB_CPU, GATEBOY_PPU] {
            column_defs(system, columns).unwrap_or_else(|e| panic!("{system}: {e}"));
        }
    }
}

#[test]
fn every_gb_adapter_emits_resolvable_columns() {
    for (adapter, systems, omitted) in GB_ADAPTERS {
        let columns: Vec<&str> = GB_CPU
            .iter()
            .chain(GB_IO)
            .copied()
            .filter(|n| !omitted.contains(n))
            .collect();
        for system in *systems {
            let defs = column_defs(system, &columns)
                .unwrap_or_else(|e| panic!("{adapter} on {system}: {e}"));
            assert_eq!(defs.len(), columns.len(), "{adapter} on {system}");
        }
    }
}

fn bgb_extension_fields() -> BTreeMap<String, ExtensionField> {
    BGB_EXTENSIONS
        .iter()
        .map(|&name| {
            let ext = ExtensionField {
                field_type: FieldType::UInt8,
                nullable: false,
                description: None,
                source: Some("bgb".into()),
                subsystem: Some("gbmicrotest".into()),
                layer: Some("result".into()),
            };
            (name.to_string(), ext)
        })
        .collect()
}

#[test]
fn bgb_result_columns_resolve_with_its_extension_declarations() {
    // The shared corpus observations already claim the plain names.
    for name in ["result", "code", "observed", "expected"] {
        assert!(column_defs("dmg", &[name]).is_ok(), "{name}");
    }
    for &name in BGB_EXTENSIONS {
        assert!(
            column_defs("dmg", &[name]).is_err(),
            "{name} is a dmg column"
        );
    }

    let mut fields: Vec<&str> = ["pc", "a", "ly"].to_vec();
    fields.extend(BGB_EXTENSIONS);
    let mut h = header("dmg", &fields);
    h.extension_fields = bgb_extension_fields();
    describe(&mut h).unwrap();
    for &name in BGB_EXTENSIONS {
        let def = h.field_def(name).unwrap();
        assert_eq!(def.field_type, FieldType::UInt8, "{name}");
        assert_eq!(def.subsystem.as_deref(), Some("gbmicrotest"), "{name}");
        assert_eq!(def.layer.as_deref(), Some("result"), "{name}");
    }

    let p = parse_profile(
        "[profile]\nname = \"t\"\ndescription = \"t\"\ntrigger = \"instruction\"\n\n[fields]\ncpu = \"registers\"\n\n[fields.extensions]\nbgb = [\"gbmicrotest_actual\", \"gbmicrotest_expected\", \"gbmicrotest_result\"]\n",
    )
    .unwrap();
    assert_eq!(p.extensions["bgb"], BGB_EXTENSIONS);
}

/// Each adapter's columns, by the system ids it writes.
const ADAPTERS: &[(&str, &[&str], &[&str])] = &[
    ("gearsystem", &["sg1000"], Z80_TI_VDP),
    ("gearcoleco", &["colecovision"], Z80_TI_VDP),
    ("ares", &["colecovision", "sg1000", "msx1"], Z80_TI_VDP),
    ("openmsx", &["msx1"], OPENMSX),
    (
        "mame (TI VDP machines)",
        &["sg1000", "colecovision"],
        MAME_Z80,
    ),
    ("mame (vcs)", &["vcs"], MAME_VCS),
    ("stella", &["vcs"], STELLA),
    ("gopher2600", &["vcs"], GOPHER2600),
    ("morepork-missingno-vcs", &["vcs"], MISSINGNO_VCS),
];

#[test]
fn every_adapter_column_resolves_for_its_system() {
    for (adapter, systems, columns) in ADAPTERS {
        for system in *systems {
            let defs = column_defs(system, columns)
                .unwrap_or_else(|e| panic!("{adapter} on {system}: {e}"));
            assert_eq!(defs.len(), columns.len(), "{adapter} on {system}");
        }
    }
}

#[test]
fn an_unknown_column_names_the_system_and_the_column() {
    let err = column_defs("sg1000", &["pc", "reg0"]).unwrap_err();
    assert!(
        matches!(&err, Error::UnknownColumn { system, column } if system == "sg1000" && column == "reg0")
    );
    assert_eq!(err.to_string(), "system 'sg1000' has no column 'reg0'");
}

#[test]
fn coleco_is_not_a_system_id() {
    assert!(matches!(
        column_defs("coleco", &["pc"]),
        Err(Error::UnknownSystem(_))
    ));
    assert!(column_defs("colecovision", &["pc"]).is_ok());
}

#[test]
fn bridge_observations_belong_to_their_system() {
    let op_addr = &column_defs("dmg", &["op_addr"]).unwrap()[0];
    assert_eq!(op_addr.field_type, FieldType::UInt16);
    assert!(column_defs("sg1000", &["op_addr"]).is_err());
    let line = &column_defs("vcs", &["line"]).unwrap()[0];
    assert_eq!(line.field_type, FieldType::UInt16);
}

#[test]
fn columns_carry_the_schema_types() {
    let defs = column_defs(
        "sg1000",
        &[
            "vdp_frame_flag",
            "vdp_line_xtal",
            "cycles",
            "ram_write_addr",
        ],
    )
    .unwrap();
    let types: Vec<FieldType> = defs.iter().map(|d| d.field_type).collect();
    assert_eq!(
        types,
        [
            FieldType::Bool,
            FieldType::UInt16,
            FieldType::UInt16,
            FieldType::UInt16
        ]
    );
    assert!(defs[3].nullable);
    assert_eq!(defs[0].subsystem.as_deref(), Some("vdp"));
}

fn header(system: &str, fields: &[&str]) -> TraceHeader {
    TraceHeader {
        _header: true,
        system: system.into(),
        fields: fields.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn describe_states_the_schema_identity() {
    let mut h = header("dmg", &["op_addr", "pc", "a"]);
    describe(&mut h).unwrap();
    assert_eq!(h.isa, "sm83");
    assert_eq!(h.entry_addrs, Some((0x0100, 0x0101)));
    assert_eq!(h.instruction_addr_field.as_deref(), Some("op_addr"));
    assert_eq!(h.field_defs.len(), 3);

    let mut h = header("colecovision", &["pc"]);
    describe(&mut h).unwrap();
    assert_eq!(h.isa, "z80");
    assert_eq!(h.entry_addrs, None);

    assert!(describe(&mut header("sg1000", &["pc", "status"])).is_err());
}

#[test]
fn a_described_header_round_trips_its_entry_addrs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.morepork");
    let mut h = header("cgb", &["op_addr", "a"]);
    describe(&mut h).unwrap();
    let mut w = morepork::format::write::MoreporkWriter::create(&path, &h, &[]).unwrap();
    w.set_u16(0, 0x0100);
    w.set_u8(1, 0x11);
    w.finish_entry().unwrap();
    w.finish().unwrap();

    let store = morepork::store::open_trace_store(&path).unwrap();
    let read = store.header();
    assert_eq!(read.entry_addrs, Some((0x0100, 0x0101)));
    assert_eq!(read.system, "cgb");
    assert_eq!(read.isa, "sm83");
}

const SG1000_PROFILE: &str = r#"
[profile]
name = "sg1000-smoke"
description = "SG-1000 CPU + VDP registers"
trigger = "instruction"
system = "sg1000"

[fields]
cpu = "registers"
vdp = "registers"
"#;

#[test]
fn profile_selections_expand_over_the_schema() {
    let p = parse_profile(SG1000_PROFILE).unwrap();
    assert_eq!(p.system, "sg1000");
    let fields: Vec<&str> = p.fields.iter().map(String::as_str).collect();
    assert!(fields.starts_with(&["a", "f", "b", "c", "d", "e", "h", "l", "a_alt"]));
    assert!(fields.contains(&"vdp_r7"));
    assert!(fields.contains(&"vdp_fifth_sprite_index"));
    // Boundary-tier fields are the `internal` layer, not `registers`.
    assert!(!fields.contains(&"wz"));
    assert!(!fields.contains(&"vdp_line"));
}

#[test]
fn profile_layers_follow_tiers_and_observations() {
    let p = parse_profile(
        r#"
[profile]
name = "t"
description = "t"
trigger = "instruction"

[fields]
cpu = ["registers", "timing"]
"#,
    )
    .unwrap();
    assert_eq!(p.system, "dmg");
    assert!(p.fields.contains(&"pc".to_string()));
    assert!(p.fields.contains(&"op_addr".to_string()));
    assert!(p.fields.contains(&"cycles".to_string()));
    assert!(!p.fields.contains(&"ime_enable_pending".to_string()));
}

#[test]
fn profile_rejections() {
    let base = |system: &str, fields: &str| {
        format!(
            "[profile]\nname = \"t\"\ndescription = \"t\"\ntrigger = \"instruction\"\nsystem = \"{system}\"\n\n[fields]\n{fields}\n"
        )
    };
    let err = parse_profile(&base("n64", "cpu = \"registers\""))
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown system 'n64'"), "{err}");
    let err = parse_profile(&base("dmg", "vdp = \"registers\""))
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown subsystem 'vdp'"), "{err}");
    let err = parse_profile(&base("dmg", "cpu = \"bogus\""))
        .unwrap_err()
        .to_string();
    assert!(err.contains("does not have layer 'bogus'"), "{err}");
    // Memory watches are gone: a memory column is an extension field.
    assert!(parse_profile(&base(
        "dmg",
        "cpu = \"registers\"\n\n[fields.memory]\nwatch = \"C000\"",
    ))
    .is_err());
    let err = parse_profile(&base(
        "dmg",
        "cpu = \"registers\"\n\n[fields.extensions]\nx = [\"ly\"]",
    ))
    .unwrap_err()
    .to_string();
    assert!(err.contains("shadows a built-in field"), "{err}");
}
