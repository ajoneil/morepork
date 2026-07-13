use morepork::*;
use std::path::PathBuf;

#[test]
fn parse_profiles() {
    let suites_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("test-suites");

    let gbmicrotest = Profile::load(suites_dir.join("gbmicrotest/profile.toml")).unwrap();
    assert_eq!(gbmicrotest.name, "gbmicrotest");
    assert!(gbmicrotest.fields.contains(&"pc".to_string()));
    assert!(gbmicrotest.fields.contains(&"lcdc".to_string()));
    assert!(gbmicrotest.fields.contains(&"vram_addr".to_string()));
    assert!(gbmicrotest.memory.contains_key("test_result"));

    let blargg = Profile::load(suites_dir.join("blargg/profile.toml")).unwrap();
    assert!(blargg.fields.contains(&"pix".to_string()));
    assert!(blargg.fields.contains(&"div".to_string()));
    // blargg has ppu registers + output but not internal/writes
    assert!(!blargg.fields.contains(&"vram_addr".to_string()));
}

#[test]
fn profile_rejects_unknown_layer() {
    let toml = r#"
[profile]
name = "bad"
description = "bad profile"
trigger = "instruction"

[fields]
cpu = "bogus_layer"
"#;
    let result = Profile::parse(toml);
    assert!(result.is_err());
}

#[test]
fn profile_layer_selection_variants() {
    // Bool true = all layers
    let toml = r#"
[profile]
name = "test"
description = "test"
trigger = "instruction"

[fields]
cpu = true
"#;
    let p = Profile::parse(toml).unwrap();
    assert!(p.fields.contains(&"pc".to_string()));
    assert!(p.fields.contains(&"ime".to_string()));
    assert!(p.fields.contains(&"mcycles".to_string()));

    // Single layer string
    let toml = r#"
[profile]
name = "test"
description = "test"
trigger = "instruction"

[fields]
cpu = "registers"
"#;
    let p = Profile::parse(toml).unwrap();
    assert!(p.fields.contains(&"pc".to_string()));
    assert!(!p.fields.contains(&"mcycles".to_string()));

    // Multiple layers
    let toml = r#"
[profile]
name = "test"
description = "test"
trigger = "tcycle"

[fields]
ppu = ["registers", "output"]
"#;
    let p = Profile::parse(toml).unwrap();
    assert!(p.fields.contains(&"lcdc".to_string()));
    assert!(p.fields.contains(&"pix".to_string()));
    assert!(!p.fields.contains(&"oam0_x".to_string()));
    assert!(!p.fields.contains(&"vram_addr".to_string()));
}

#[test]
fn entry_hex_formatting() {
    let mut e = TraceEntry::new();
    e.set_u8("a", 0x0F);
    e.set_u8("f", 0x00);
    e.set_u16("pc", 0x0100);
    e.set_u16("sp", 0xFFFF);
    e.set_bool("ime", true);

    assert_eq!(e.get("a").unwrap().as_u64().unwrap(), 0x0F);
    assert_eq!(e.get("f").unwrap().as_u64().unwrap(), 0x00);
    assert_eq!(e.get("pc").unwrap().as_u64().unwrap(), 0x0100);
    assert_eq!(e.get("sp").unwrap().as_u64().unwrap(), 0xFFFF);
    assert!(e.get("ime").unwrap().as_bool().unwrap());
}

#[test]
fn header_validation() {
    let h = TraceHeader {
        _header: false,
        format_version: "0.1.0".into(),
        emulator: "test".into(),
        emulator_version: "1.0".into(),
        rom_sha256: "0000".into(),
        model: "DMG".into(),
        boot_rom: BootRom::Skip,
        profile: "test".into(),
        fields: vec!["pc".into()],
        trigger: Trigger::Instruction,
        pix_format: PixFormat::default(),
        extension_fields: std::collections::BTreeMap::new(),
        notes: String::new(),
        ..Default::default()
    };
    assert!(h.validate().is_err());

    // Empty `fields` is permitted at validate time — JSONL inputs may infer
    // fields from the first data line, so the construction-time check would
    // be too strict. Field-list emptiness shows up later as a no-op trace.
    let h = TraceHeader {
        _header: true,
        format_version: "0.1.0".into(),
        emulator: "test".into(),
        emulator_version: "1.0".into(),
        rom_sha256: "0000".into(),
        model: "DMG".into(),
        boot_rom: BootRom::Skip,
        profile: "test".into(),
        fields: vec![],
        trigger: Trigger::Instruction,
        pix_format: PixFormat::default(),
        extension_fields: std::collections::BTreeMap::new(),
        notes: String::new(),
        ..Default::default()
    };
    assert!(h.validate().is_ok());
}

#[test]
fn profile_family_defaults_to_gb() {
    let toml = r#"
[profile]
name = "t"
description = "t"
trigger = "instruction"

[fields]
cpu = "registers"
"#;
    let p = Profile::parse(toml).unwrap();
    assert_eq!(p.family, "gb");
    assert!(p.fields.contains(&"pc".to_string()));
}

#[test]
fn profile_rejects_unknown_family() {
    let toml = r#"
[profile]
name = "t"
description = "t"
trigger = "instruction"
family = "n64"

[fields]
cpu = "registers"
"#;
    let err = Profile::parse(toml).unwrap_err().to_string();
    assert!(err.contains("unknown family 'n64'"), "{err}");
}

#[test]
fn profile_rejects_unknown_subsystem() {
    let toml = r#"
[profile]
name = "t"
description = "t"
trigger = "instruction"

[fields]
vdp = "registers"
"#;
    let err = Profile::parse(toml).unwrap_err().to_string();
    assert!(err.contains("unknown subsystem 'vdp'"), "{err}");
}

#[test]
fn parse_every_suite_profile() {
    let suites_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("test-suites");
    let mut parsed = 0;
    for entry in std::fs::read_dir(&suites_dir).unwrap() {
        let profile_path = entry.unwrap().path().join("profile.toml");
        if profile_path.exists() {
            let p = Profile::load(&profile_path)
                .unwrap_or_else(|e| panic!("{}: {e}", profile_path.display()));
            assert_eq!(p.family, "gb", "{}", profile_path.display());
            parsed += 1;
        }
    }
    assert!(parsed >= 10, "expected all suite profiles, parsed {parsed}");
}

#[test]
fn nes_profile_and_flag_queries() {
    let toml = r#"
[profile]
name = "nes-smoke"
description = "NES CPU + PPU registers"
trigger = "instruction"
family = "nes"

[fields]
cpu = "registers"
ppu = "registers"
"#;
    let p = Profile::parse(toml).unwrap();
    assert_eq!(p.family, "nes");
    assert_eq!(
        p.fields,
        ["pc", "a", "x", "y", "s", "p", "control", "mask", "line", "dot"]
            .map(String::from)
    );

    // Flag vocabulary resolves against P, not the GB F register.
    let nes = morepork::family::family("nes").unwrap();
    let cond = morepork::query::parse_condition("flag n becomes set", nes).unwrap();
    match cond {
        morepork::query::Condition::BitTransition { field, bit, to } => {
            assert_eq!((field.as_str(), bit, to), ("p", 7, true));
        }
        other => panic!("unexpected condition: {other:?}"),
    }
    // GB phrases are not in the NES vocabulary.
    assert!(morepork::query::parse_condition("lcd on", nes).is_err());
    assert!(morepork::query::parse_condition("flag h set", nes).is_err());
}

#[test]
fn labelled_phrases_parse_in_their_family() {
    for family in morepork::family::FAMILIES {
        for chip in family.labelled_phrases {
            morepork::query::parse_condition(chip.query, family).unwrap_or_else(|e| {
                panic!(
                    "family '{}' chip '{}' has unparseable query '{}': {e}",
                    family.id, chip.label, chip.query
                )
            });
            assert!(
                family.lookup_field(chip.needs).is_some(),
                "family '{}' chip '{}' needs unknown field '{}'",
                family.id, chip.label, chip.needs
            );
        }
    }
}

#[test]
fn vcs_profile_and_flag_queries() {
    let toml = r#"
[profile]
name = "vcs-smoke"
description = "6507 + TIA beam + RIOT"
trigger = "instruction"
family = "vcs"

[fields]
cpu = "registers"
tia = "registers"
riot = "registers"
"#;
    let p = Profile::parse(toml).unwrap();
    assert_eq!(p.family, "vcs");
    assert_eq!(
        p.fields,
        ["pc", "a", "x", "y", "s", "p", "line", "clock", "timer", "port_a", "port_b"]
            .map(String::from)
    );

    // The 6502 flag vocabulary is shared with the NES family.
    let vcs = morepork::family::family("vcs").unwrap();
    let cond = morepork::query::parse_condition("flag c set", vcs).unwrap();
    match cond {
        morepork::query::Condition::FieldBitMask { field, mask } => {
            assert_eq!((field.as_str(), mask), ("p", 1));
        }
        other => panic!("unexpected condition: {other:?}"),
    }
    // Phrases from the other families are not in the VCS vocabulary.
    assert!(morepork::query::parse_condition("lcd on", vcs).is_err());
    assert!(morepork::query::parse_condition("vblank starts", vcs).is_err());
}
