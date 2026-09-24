use morepork::*;

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

/// A self-describing header stating a system and ISA, with one u8 column.
fn header(system: &str, isa: &str) -> TraceHeader {
    TraceHeader {
        _header: true,
        system: system.into(),
        isa: isa.into(),
        fields: vec!["a".into()],
        field_defs: vec![morepork::header::HeaderFieldDef {
            name: "a".into(),
            field_type: FieldType::UInt8,
            subsystem: Some("cpu".into()),
            layer: Some("registers".into()),
            nullable: false,
            dictionary: false,
            source: None,
        }],
        ..Default::default()
    }
}

#[test]
fn system_is_read_from_the_header() {
    let h = header("colecovision", "z80");
    let sys = h.system_def().unwrap();
    assert_eq!(sys.id, "colecovision");
    assert_eq!(sys.isa.id, "z80");
    assert_eq!(sys.entry_addrs, None);
}

#[test]
fn system_requires_field_defs_and_a_known_isa() {
    let mut h = header("sg1000", "z80");
    h.field_defs.clear();
    assert!(h.system_def().is_err());

    let err = header("n64", "mips").system_def().err().unwrap().to_string();
    assert!(err.contains("unknown ISA 'mips'"), "{err}");
}

#[test]
fn z80_flag_queries_and_ti_vdp_phrases() {
    let h = header("sg1000", "z80");
    let sg = h.system_def().unwrap();

    // Flag vocabulary resolves against the Z80 F register, including the
    // undocumented X/Y bits and the P/V aliases.
    let cond = morepork::query::parse_condition("flag s becomes set", &sg).unwrap();
    match cond {
        morepork::query::Condition::BitTransition { field, bit, to } => {
            assert_eq!((field.as_str(), bit, to), ("f", 7, true));
        }
        other => panic!("unexpected condition: {other:?}"),
    }
    let cond = morepork::query::parse_condition("flag pv set", &sg).unwrap();
    match cond {
        morepork::query::Condition::FieldBitMask { field, mask } => {
            assert_eq!((field.as_str(), mask), ("f", 1 << 2));
        }
        other => panic!("unexpected condition: {other:?}"),
    }

    // "vblank starts" desugars to the VDP's line-192 transition, for every
    // system on the TI VDP.
    for id in ["sg1000", "colecovision", "msx1"] {
        let h = header(id, "z80");
        let sys = h.system_def().unwrap();
        let cond = morepork::query::parse_condition("vblank starts", &sys).unwrap();
        match cond {
            morepork::query::Condition::FieldChangesTo { field, value } => {
                assert_eq!((field.as_str(), value.as_str()), ("vdp_line", "0xc0"), "{id}");
            }
            other => panic!("unexpected condition: {other:?}"),
        }
    }

    // GB phrases are not in the SG-1000 vocabulary.
    assert!(morepork::query::parse_condition("lcd on", &sg).is_err());
}

#[test]
fn mos6502_flag_queries() {
    let h = header("vcs", "6502");
    let vcs = h.system_def().unwrap();
    let cond = morepork::query::parse_condition("flag c set", &vcs).unwrap();
    match cond {
        morepork::query::Condition::FieldBitMask { field, mask } => {
            assert_eq!((field.as_str(), mask), ("p", 1));
        }
        other => panic!("unexpected condition: {other:?}"),
    }
    // Phrases from the other systems are not in the VCS vocabulary.
    assert!(morepork::query::parse_condition("lcd on", &vcs).is_err());
    assert!(morepork::query::parse_condition("vblank starts", &vcs).is_err());
    assert!(morepork::query::parse_condition("flag h set", &vcs).is_err());
}

#[test]
fn gb_phrases_follow_the_system_id() {
    let h = header("dmg", "sm83");
    let dmg = h.system_def().unwrap();
    assert!(morepork::query::parse_condition("lcd on", &dmg).is_ok());
    assert!(morepork::query::parse_condition("interrupt 2", &dmg).is_ok());
    assert!(morepork::query::parse_condition("flag h set", &dmg).is_ok());
}

#[test]
fn header_round_trip_carries_entry_addrs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.morepork");
    let mut h = header("dmg", "sm83");
    h.entry_addrs = Some((0x0100, 0x0101));
    let mut w = morepork::format::write::MoreporkWriter::create(&path, &h, &[]).unwrap();
    w.set_u8(0, 1);
    w.finish_entry().unwrap();
    w.finish().unwrap();

    let store = morepork::store::open_trace_store(&path).unwrap();
    assert_eq!(store.header().entry_addrs, Some((0x0100, 0x0101)));
    assert_eq!(store.header().system_def().unwrap().entry_addrs, Some((0x0100, 0x0101)));
}

#[test]
fn writer_rejects_an_undeclared_column() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = header("sg1000", "z80");
    h.fields.push("b".into());
    assert!(morepork::format::write::MoreporkWriter::create(dir.path().join("t.morepork"), &h, &[]).is_err());
}

// --- ISA-driven disassembly through the shared missingno_core vocabulary ---

use missingno_core::isa::{Flow, Instruction, InstructionSet, OperandClass};

/// A minimal SM83 front end over the shared trait, standing in for the real
/// `missingno_gb::Sm83` (whose crate cannot be a morepork dependency without
/// a cycle). Enough opcodes to prove the render path decodes through the trait.
struct ToySm83;

impl InstructionSet for ToySm83 {
    fn max_len(&self) -> usize {
        3
    }
    fn decode(&self, _address: u32, bytes: &[u8]) -> Instruction {
        match bytes.first().copied().unwrap_or(0) {
            0x00 => Instruction { mnemonic: "nop".into(), length: 1, flow: Flow::Sequential },
            0x01 => {
                let word = u16::from_le_bytes([bytes[1], bytes[2]]);
                Instruction {
                    mnemonic: format!("ld bc,${word:04x}"),
                    length: 3,
                    flow: Flow::Sequential,
                }
            }
            other => Instruction {
                mnemonic: format!("${other:02x}"),
                length: 1,
                flow: Flow::Sequential,
            },
        }
    }
    fn classify_operand(&self, _operand: &str) -> OperandClass {
        OperandClass::Plain
    }
}

#[test]
fn disassemble_sm83_through_shared_isa() {
    // nop ; ld bc,$1234 ; nop
    let rom = [0x00u8, 0x01, 0x34, 0x12, 0x00];
    let rows = morepork::disasm::disassemble_rows(&ToySm83, &rom, 0, 3);
    assert_eq!(
        rows,
        vec![
            (0, "nop".to_string()),
            (1, "ld bc,$1234".to_string()),
            (4, "nop".to_string()),
        ]
    );
}

#[test]
fn disassemble_6502_through_real_decoder() {
    // The real missingno-mos-6502 decoder, driven by the same shared trait.
    let isa = missingno_mos_6502::Mos6502;
    // lda #$7f ; sta $02 ; jmp $8000
    let rom = [0xA9u8, 0x7F, 0x85, 0x02, 0x4C, 0x00, 0x80];
    let rows = morepork::disasm::disassemble_rows(&isa, &rom, 0, 3);
    assert_eq!(rows[0], (0, "lda #$7f".to_string()));
    assert_eq!(rows[1], (2, "sta $02".to_string()));
    assert_eq!(rows[2], (4, "jmp $8000".to_string()));
}
