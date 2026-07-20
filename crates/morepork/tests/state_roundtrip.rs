//! The state-file framing round-trips through morepork's re-exported API over
//! the shared `missingno_core` state vocabulary: a full-state record plus memory
//! spans and a frame, written and read back against an authored schema.

use missingno_core::state::{
    FieldDef, FieldType, FrameSpec, MemorySpan, PixelFormat, StateRecord, StateValue,
    SystemStateSchema,
};
use morepork::snapshot::{StateMeta, read_state_file, write_state_file};

fn schema() -> SystemStateSchema {
    SystemStateSchema {
        system: "dmg",
        fields: vec![
            FieldDef::observable("a", FieldType::U8, "cpu"),
            FieldDef::observable("pc", FieldType::U16, "cpu"),
            FieldDef::boundary("mbc_type", FieldType::Str, "cartridge"),
            FieldDef::boundary("ime", FieldType::Bool, "cpu"),
        ],
        memory: vec![MemorySpan::addressable("wram", 0xC000, 0x2000)],
        frame: FrameSpec {
            width: 160,
            height: Some(144),
            format: PixelFormat::Shade2,
        },
    }
}

#[test]
fn state_file_round_trips_through_morepork() {
    let schema = schema();

    let mut record = StateRecord::new();
    record
        .set("a", 0x42u8)
        .set("pc", 0x0150u16)
        .set("mbc_type", "mbc1")
        .set("ime", true);

    let meta = StateMeta {
        system: "dmg",
        rom_sha256: Some("deadbeef"),
        emulator: "missingno",
        emulator_version: "0.0.1",
    };
    let memory = vec![("wram", vec![0x7Eu8; 0x2000])];
    let bytes = write_state_file(&meta, &record, &memory, None);

    let file = read_state_file(&bytes).expect("morepork reads the state file");
    assert_eq!(file.system, "dmg");
    assert_eq!(file.rom_sha256.as_deref(), Some("deadbeef"));
    assert_eq!(file.memory[0].0, "wram");
    assert_eq!(file.memory[0].1.len(), 0x2000);

    // The record rebuilds and validates against the shared schema.
    let rebuilt = schema
        .record_from(file.fields)
        .expect("the record validates");
    assert_eq!(rebuilt.get("pc"), Some(&StateValue::Int(0x0150)));
    assert_eq!(
        rebuilt.get("mbc_type"),
        Some(&StateValue::Text("mbc1".into()))
    );
}
