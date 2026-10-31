// SPDX-License-Identifier: AGPL-3.0-only

use botw_save_convert::Platform;
use botw_save_convert::save::{
    ConvertOptions, ConvertReport, SaveInfo, SlotAction, convert_save, default_output_path,
    inspect_save,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Header versions used by the fixtures.
const V15: u16 = 0x471b;
const V16: u16 = 0x471e;
const V19: u16 = 0x4730;

/// A field that only exists from v1.6 on.
const V16_FIELD: u32 = 0xf331_6abb;
/// A `game_data.sav` field that only exists in the v1.8/v1.9 layout.
const V19_FIELD: u32 = 0x6213_3c14;
/// An `option.sav` field that only exists in the v1.8/v1.9 layout.
const V19_OPTION_FIELD: u32 = 0x1c8a_3035;

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "botw-save-convert-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn canonical(&self) -> PathBuf {
        fs::canonicalize(&self.path).unwrap()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn convert(input: &Path, output: Option<&Path>, options: &ConvertOptions) -> ConvertReport {
    let info = inspect_save(input).unwrap();
    let output = match output {
        Some(path) => path.to_path_buf(),
        None => default_output_path(&info.root, info.target()).unwrap(),
    };

    convert_save(&info, &output, options).expect("conversion failed")
}

fn try_convert(
    input: &Path,
    output: &Path,
    options: &ConvertOptions,
) -> botw_save_convert::Result<ConvertReport> {
    let info = inspect_save(input)?;
    convert_save(&info, output, options)
}

fn push_word(out: &mut Vec<u8>, value: u32, big_endian: bool) {
    if big_endian {
        out.extend_from_slice(&value.to_be_bytes());
    } else {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

/// Builds the record-table container the game uses for option.sav,
/// game_data.sav and caption.sav.
fn record_table(version: u16, platform: Platform, records: &[(u32, u32)]) -> Vec<u8> {
    let big_endian = platform == Platform::WiiU;
    let mut out = Vec::new();

    push_word(&mut out, u32::from(version), big_endian);
    push_word(&mut out, 0xffff_ffff, big_endian);
    push_word(&mut out, 1, big_endian);
    for (hash, value) in records {
        push_word(&mut out, *hash, big_endian);
        push_word(&mut out, *value, big_endian);
    }
    push_word(&mut out, 0xffff_ffff, big_endian);

    out
}

fn make_save(root: &Path, unknown_version: bool) -> Vec<u8> {
    fs::create_dir_all(root.join("0")).unwrap();

    fs::write(
        root.join("option.sav"),
        record_table(V15, Platform::WiiU, &[(0x1111, 1), (0x2222, 2)]),
    )
    .unwrap();

    let game_data = if unknown_version {
        let mut data = vec![0, 0, 0, 0];
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);
        data
    } else {
        record_table(V15, Platform::WiiU, &[(0xaaaa, 7), (0xbbbb, 9)])
    };
    fs::write(root.join("0/game_data.sav"), &game_data).unwrap();

    fs::write(
        root.join("0/trackblock_0.sav"),
        [
            0x00, 0x00, 0x00, 0x00, 0xaa, 0xbb, 0xcc, 0xdd, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
            0x07, 0x08,
        ],
    )
    .unwrap();

    game_data
}

/// A save from the v1.8/v1.9 update: every record table uses the 0x4730 header
/// carries the fields that only exist after v1.5.
fn make_newer_save(root: &Path) {
    fs::create_dir_all(root.join("7")).unwrap();

    fs::write(
        root.join("option.sav"),
        record_table(
            V19,
            Platform::Switch,
            &[(0x1111, 1), (0x2222, 2), (V19_OPTION_FIELD, 5)],
        ),
    )
    .unwrap();

    fs::write(
        root.join("7/game_data.sav"),
        record_table(
            V19,
            Platform::Switch,
            &[(0xaaaa, 7), (V16_FIELD, 1), (V19_FIELD, 3)],
        ),
    )
    .unwrap();
}

#[test]
fn converts_a_wii_u_save_and_leaves_the_source_untouched() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    let output = temporary.path().join("converted");
    let original_game_data = make_save(&input, false);
    let original_option = fs::read(input.join("option.sav")).unwrap();

    let report = convert(&input, Some(&output), &ConvertOptions::default());

    assert_eq!(report.target, Platform::Switch);
    assert_eq!(report.files_converted, 3);
    assert_eq!(report.files_copied, 3);
    assert_eq!(report.files_kept, 0);
    assert_eq!(report.files_downgraded, 0);

    assert_eq!(fs::read(input.join("option.sav")).unwrap(), original_option);
    assert_eq!(
        fs::read(input.join("0/game_data.sav")).unwrap(),
        original_game_data
    );
    assert_eq!(
        fs::read(output.join("option.sav")).unwrap()[..4],
        Platform::Switch.magic()
    );

    let converted = fs::read(output.join("0/game_data.sav")).unwrap();
    assert_eq!(
        converted,
        record_table(V15, Platform::Switch, &[(0xaaaa, 7), (0xbbbb, 9)]),
        "the record table must only change byte order"
    );

    let inspection = inspect_save(&output).unwrap();
    assert_eq!(inspection.platform, Platform::Switch);
    assert_eq!(inspection.version_summary(), "v1.5 x1");
}

#[test]
fn converts_back_to_wii_u_byte_for_byte() {
    let temporary = TestDirectory::new();
    let wii_u = temporary.path().join("wiiu");
    let switch = temporary.path().join("switch");
    let round_trip = temporary.path().join("round-trip");
    make_save(&wii_u, false);
    let original_option = fs::read(wii_u.join("option.sav")).unwrap();
    let original_game_data = fs::read(wii_u.join("0/game_data.sav")).unwrap();
    let original_trackblock = fs::read(wii_u.join("0/trackblock_0.sav")).unwrap();

    convert(&wii_u, Some(&switch), &ConvertOptions::default());
    let report = convert(&switch, Some(&round_trip), &ConvertOptions::default());

    assert_eq!(report.target, Platform::WiiU);
    assert_eq!(
        fs::read(round_trip.join("option.sav")).unwrap(),
        original_option
    );
    assert_eq!(
        fs::read(round_trip.join("0/game_data.sav")).unwrap(),
        original_game_data
    );
    assert_eq!(
        fs::read(round_trip.join("0/trackblock_0.sav")).unwrap(),
        original_trackblock
    );
}

#[test]
fn downgrades_a_newer_save_to_the_v1_5_layout() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("switch2");
    let output = temporary.path().join("wiiu");
    make_newer_save(&input);
    let source_size = fs::metadata(input.join("7/game_data.sav")).unwrap().len();

    let info = inspect_save(&input).unwrap();
    assert_eq!(info.platform, Platform::Switch);
    assert_eq!(info.version_summary(), "v1.8/v1.9 x1");
    assert!(info.blockers(&ConvertOptions::default()).is_empty());

    let report = convert_save(&info, &output, &ConvertOptions::default()).unwrap();

    assert_eq!(report.target, Platform::WiiU);
    assert_eq!(report.files_downgraded, 2, "option.sav and slot 7");
    assert_eq!(report.removed_fields, 3, "two in game_data, one in option");

    let converted = fs::read(output.join("7/game_data.sav")).unwrap();
    assert_eq!(&converted[..4], &Platform::WiiU.magic());
    assert_eq!(
        converted,
        record_table(V15, Platform::WiiU, &[(0xaaaa, 7)]),
        "both newer records must be gone"
    );
    assert_eq!(
        converted.len() as u64,
        source_size - 16,
        "two records must be gone"
    );

    let option = fs::read(output.join("option.sav")).unwrap();
    assert_eq!(&option[..4], &Platform::WiiU.magic());
    assert_eq!(
        option,
        record_table(V15, Platform::WiiU, &[(0x1111, 1), (0x2222, 2)])
    );

    let inspection = inspect_save(&output).unwrap();
    assert_eq!(inspection.platform, Platform::WiiU);
    assert_eq!(inspection.version_summary(), "v1.5 x1");
    assert_eq!(inspection.option_label(), "v1.5");
}

#[test]
fn downgrades_a_v1_6_save_to_v1_5() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("switch");
    let output = temporary.path().join("wiiu");
    fs::create_dir_all(input.join("0")).unwrap();

    fs::write(
        input.join("option.sav"),
        record_table(V15, Platform::Switch, &[(0x1111, 1)]),
    )
    .unwrap();
    fs::write(
        input.join("0/game_data.sav"),
        record_table(V16, Platform::Switch, &[(0xaaaa, 7), (V16_FIELD, 1)]),
    )
    .unwrap();

    let report = convert(&input, Some(&output), &ConvertOptions::default());

    assert_eq!(report.files_downgraded, 1);
    assert_eq!(report.removed_fields, 1);

    let converted = fs::read(output.join("0/game_data.sav")).unwrap();
    assert_eq!(&converted[..4], &Platform::WiiU.magic());
    assert!(
        !converted
            .windows(4)
            .any(|window| window == V16_FIELD.to_be_bytes())
    );
}

#[test]
fn refuses_unknown_versions_without_the_explicit_flag() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    let output = temporary.path().join("converted");
    make_save(&input, true);

    let error = try_convert(&input, &output, &ConvertOptions::default()).unwrap_err();
    assert!(error.to_string().contains("--allow-unsupported"));
    assert!(!output.exists());

    let options = ConvertOptions {
        allow_unsupported: true,
        ..ConvertOptions::default()
    };
    let report = convert(&input, Some(&output), &options);
    assert_eq!(report.files_converted, 3);
    assert!(output.exists());
}

#[test]
fn refuses_to_overwrite_an_existing_output_directory() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    let output = temporary.path().join("converted");
    make_save(&input, false);
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep-me"), b"do not overwrite").unwrap();

    let error = try_convert(&input, &output, &ConvertOptions::default()).unwrap_err();
    assert!(error.to_string().contains("already exists"));
    assert_eq!(
        fs::read(output.join("keep-me")).unwrap(),
        b"do not overwrite"
    );
}

#[test]
fn refuses_an_output_directory_inside_the_input() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    let output = input.join("converted");
    make_save(&input, false);

    let error = try_convert(&input, &output, &ConvertOptions::default()).unwrap_err();
    assert!(error.to_string().contains("cannot be inside the input"));
    assert!(!output.exists());
}

#[test]
fn default_output_is_a_sibling_named_after_the_target_platform() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    make_save(&input, false);

    let report = convert(&input, None, &ConvertOptions::default());

    assert_eq!(
        report.output,
        temporary.canonical().join("save-switch"),
        "unexpected default output"
    );
    assert!(report.output.join("option.sav").is_file());
    assert!(input.join("option.sav").is_file());
}

#[test]
fn default_output_replaces_an_existing_platform_suffix() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save-wiiu");
    make_save(&input, false);

    let report = convert(&input, None, &ConvertOptions::default());

    assert_eq!(report.output, temporary.canonical().join("save-switch"));
}

#[test]
fn creates_missing_parent_directories() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    let output = temporary.path().join("a/b/c/converted");
    make_save(&input, false);

    let report = convert(&input, Some(&output), &ConvertOptions::default());

    assert_eq!(report.output, temporary.canonical().join("a/b/c/converted"));
    assert!(report.output.join("option.sav").is_file());
}

#[test]
fn copies_slots_that_are_already_in_the_target_format() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    let output = temporary.path().join("converted");
    make_save(&input, false);

    // A slot merged from a Switch save: already little-endian, must not be
    // swapped a second time.
    fs::create_dir_all(input.join("1")).unwrap();
    let already_switch = record_table(V16, Platform::Switch, &[(0xcccc, 4)]);
    fs::write(input.join("1/game_data.sav"), &already_switch).unwrap();

    let info = inspect_save(&input).unwrap();
    assert_eq!(info.foreign_slots.len(), 1);
    assert!(info.unknown_versions.is_empty());

    let report = convert(&input, Some(&output), &ConvertOptions::default());
    assert_eq!(report.files_kept, 1);
    assert_eq!(report.files_converted, 3);
    assert_eq!(
        fs::read(output.join("1/game_data.sav")).unwrap(),
        already_switch
    );
}

#[test]
fn reports_per_slot_actions_for_a_mixed_save() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    make_save(&input, false);
    fs::create_dir_all(input.join("1")).unwrap();
    fs::write(
        input.join("1/game_data.sav"),
        record_table(V16, Platform::Switch, &[(0xcccc, 4)]),
    )
    .unwrap();

    let info = inspect_save(&input).unwrap();
    let options = ConvertOptions::default();

    let slot0 = info
        .slots
        .iter()
        .find(|slot| slot.directory == "0")
        .unwrap();
    let slot1 = info
        .slots
        .iter()
        .find(|slot| slot.directory == "1")
        .unwrap();

    assert_eq!(info.slot_action(slot0, &options), SlotAction::Convert);
    assert_eq!(info.slot_action(slot1, &options), SlotAction::Copy);
    assert_eq!(info.version_summary(), "v1.5 x1, v1.6 x1");
    assert!(info.file_breakdown().starts_with("2 slots"));
}

#[test]
fn keeps_a_supplied_wii_u_option_file() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("switch2");
    let output = temporary.path().join("wiiu");
    let template = temporary.path().join("wiiu-template");
    make_newer_save(&input);

    fs::create_dir_all(&template).unwrap();
    let wii_u_option = record_table(V15, Platform::WiiU, &[(0x1111, 42)]);
    fs::write(template.join("option.sav"), &wii_u_option).unwrap();

    let options = ConvertOptions {
        option_sav: Some(template.join("option.sav")),
        ..ConvertOptions::default()
    };

    let info = inspect_save(&input).unwrap();
    assert!(info.slots_to_drop(&options).is_empty());

    let report = convert_save(&info, &output, &options).unwrap();

    assert_eq!(report.target, Platform::WiiU);
    assert!(report.option_replaced);
    assert_eq!(fs::read(output.join("option.sav")).unwrap(), wii_u_option);
    assert_eq!(
        fs::read(output.join("7/game_data.sav")).unwrap()[..4],
        Platform::WiiU.magic()
    );
}

#[test]
fn rejects_an_option_file_from_the_wrong_platform() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("switch2");
    let output = temporary.path().join("wiiu");
    make_newer_save(&input);

    let options = ConvertOptions {
        option_sav: Some(input.join("option.sav")),
        ..ConvertOptions::default()
    };

    let info = inspect_save(&input).unwrap();
    let error = convert_save(&info, &output, &options).unwrap_err();
    assert!(
        error.to_string().contains("is a Switch option.sav"),
        "{error}"
    );
    assert!(!output.exists());
}

#[test]
fn refuses_an_unknown_newer_version_until_asked() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("future");
    let output = temporary.path().join("wiiu");
    fs::create_dir_all(input.join("0")).unwrap();
    fs::write(
        input.join("option.sav"),
        record_table(V15, Platform::Switch, &[(0x1111, 1)]),
    )
    .unwrap();
    fs::write(
        input.join("0/game_data.sav"),
        record_table(0x4740, Platform::Switch, &[(0xaaaa, 7), (V19_FIELD, 3)]),
    )
    .unwrap();

    let info = inspect_save(&input).unwrap();
    assert_eq!(info.version_summary(), "unknown 0x4740 x1");

    let error = try_convert(&input, &output, &ConvertOptions::default()).unwrap_err();
    assert!(error.to_string().contains("--allow-unsupported"), "{error}");
    assert!(!output.exists());

    let options = ConvertOptions {
        allow_unsupported: true,
        ..ConvertOptions::default()
    };
    let report = convert(&input, Some(&output), &options);
    assert_eq!(report.files_downgraded, 1);

    let converted = fs::read(output.join("0/game_data.sav")).unwrap();
    assert_eq!(&converted[..4], &Platform::WiiU.magic());
    assert_eq!(converted, record_table(V15, Platform::WiiU, &[(0xaaaa, 7)]));
}

#[test]
fn leaves_out_a_slot_with_an_unknown_newer_version() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("future");
    let output = temporary.path().join("wiiu");
    fs::create_dir_all(input.join("0")).unwrap();
    fs::create_dir_all(input.join("1")).unwrap();
    fs::write(
        input.join("option.sav"),
        record_table(V15, Platform::Switch, &[(0x1111, 1)]),
    )
    .unwrap();
    fs::write(
        input.join("0/game_data.sav"),
        record_table(V15, Platform::Switch, &[(0xaaaa, 7)]),
    )
    .unwrap();
    fs::write(
        input.join("1/game_data.sav"),
        record_table(0x4740, Platform::Switch, &[(0xbbbb, 7)]),
    )
    .unwrap();

    let options = ConvertOptions {
        drop_incompatible: true,
        ..ConvertOptions::default()
    };
    let info = inspect_save(&input).unwrap();
    let report = convert_save(&info, &output, &options).unwrap();

    assert_eq!(report.dropped, vec![PathBuf::from("1")]);
    assert!(output.join("0/game_data.sav").is_file());
    assert!(!output.join("1").exists());
}

#[test]
fn fails_loudly_when_a_newer_file_is_not_a_record_table() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("broken");
    let output = temporary.path().join("wiiu");
    fs::create_dir_all(input.join("0")).unwrap();
    fs::write(
        input.join("option.sav"),
        record_table(V15, Platform::Switch, &[(0x1111, 1)]),
    )
    .unwrap();

    let mut broken = vec![0x30, 0x47, 0x00, 0x00];
    broken.extend_from_slice(&[0x11; 40]);
    fs::write(input.join("0/game_data.sav"), &broken).unwrap();

    let error = try_convert(&input, &output, &ConvertOptions::default()).unwrap_err();
    assert!(
        error.to_string().contains("record table could not be read"),
        "{error}"
    );
    assert!(!output.exists());
}

#[test]
fn cli_help_and_usage_errors_have_the_expected_exit_codes() {
    let help = ProcessCommand::new(env!("CARGO_BIN_EXE_botw-convert"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage:"));

    let missing = ProcessCommand::new(env!("CARGO_BIN_EXE_botw-convert"))
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("expected 1 or 2 paths"));
}

#[test]
fn cli_converts_and_dry_run_writes_nothing() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    let dry_output = temporary.path().join("dry");
    let output = temporary.path().join("converted");
    make_save(&input, false);

    let dry_run = ProcessCommand::new(env!("CARGO_BIN_EXE_botw-convert"))
        .arg("--dry-run")
        .arg(&input)
        .arg(&dry_output)
        .output()
        .unwrap();
    assert!(dry_run.status.success());
    let stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(
        stdout.contains("Save type: Wii U save"),
        "unexpected summary: {stdout}"
    );
    assert!(stdout.contains("Target   : Switch (NS)"), "{stdout}");
    assert!(stdout.contains("v1.5 x1"), "{stdout}");
    assert!(stdout.contains("Dry run"));
    assert!(!dry_output.exists());

    let conversion = ProcessCommand::new(env!("CARGO_BIN_EXE_botw-convert"))
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        conversion.status.success(),
        "{}",
        String::from_utf8_lossy(&conversion.stderr)
    );
    assert!(output.join("option.sav").is_file());
}

#[test]
fn cli_dry_run_tolerates_an_existing_output_folder() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    let output = temporary.path().join("converted");
    make_save(&input, false);
    fs::create_dir(&output).unwrap();

    let dry_run = ProcessCommand::new(env!("CARGO_BIN_EXE_botw-convert"))
        .arg("--dry-run")
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();

    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    assert!(
        String::from_utf8_lossy(&dry_run.stderr).contains("already exists"),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    assert!(String::from_utf8_lossy(&dry_run.stdout).contains("Dry run"));
    assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
}

#[test]
fn cli_downgrades_a_switch_2_edition_save() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("switch2");
    let output = temporary.path().join("wiiu");
    make_newer_save(&input);

    let conversion = ProcessCommand::new(env!("CARGO_BIN_EXE_botw-convert"))
        .arg(&input)
        .arg(&output)
        .output()
        .unwrap();

    assert!(
        conversion.status.success(),
        "{}",
        String::from_utf8_lossy(&conversion.stderr)
    );
    let stdout = String::from_utf8_lossy(&conversion.stdout);
    assert!(stdout.contains("Downgraded"), "{stdout}");
    assert!(stdout.contains("v1.5"), "{stdout}");

    let converted = fs::read(output.join("7/game_data.sav")).unwrap();
    assert_eq!(&converted[..4], &Platform::WiiU.magic());
}

#[test]
fn cli_uses_the_default_output_when_only_the_input_is_given() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    make_save(&input, false);

    let conversion = ProcessCommand::new(env!("CARGO_BIN_EXE_botw-convert"))
        .arg(&input)
        .output()
        .unwrap();
    assert!(
        conversion.status.success(),
        "{}",
        String::from_utf8_lossy(&conversion.stderr)
    );

    let expected = temporary.canonical().join("save-switch");
    assert!(expected.join("option.sav").is_file());
    assert!(
        String::from_utf8_lossy(&conversion.stdout).contains("save-switch"),
        "{}",
        String::from_utf8_lossy(&conversion.stdout)
    );
}

#[test]
fn save_info_breakdown_counts_duplicate_copies() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    make_save(&input, false);
    fs::write(
        input.join("0/game_data-NSConflict-5-mac15.6.sav"),
        record_table(V16, Platform::WiiU, &[(0xdddd, 1)]),
    )
    .unwrap();

    let info: SaveInfo = inspect_save(&input).unwrap();
    assert_eq!(info.duplicates.len(), 1);
    assert_eq!(info.sav_files.len(), 4);
    assert!(info.file_breakdown().contains("1 duplicate copies"));
    assert!(info.sav_files.iter().any(|path| {
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .contains("NSConflict")
    }));
}

/// `caption.sav` stores strings as one record per four bytes, the hash
/// repeating, and those bytes keep their order on both platforms.
fn caption_table(
    platform: Platform,
    records: &[(u32, u32)],
    strings: &[(u32, &str, usize)],
) -> Vec<u8> {
    let big_endian = platform == Platform::WiiU;
    let mut out = Vec::new();

    push_word(&mut out, u32::from(V15), big_endian);
    push_word(&mut out, 0xffff_ffff, big_endian);
    push_word(&mut out, 1, big_endian);
    for (hash, value) in records {
        push_word(&mut out, *hash, big_endian);
        push_word(&mut out, *value, big_endian);
    }
    for (hash, text, chunks) in strings {
        let mut bytes = text.as_bytes().to_vec();
        bytes.resize(chunks * 4, 0);
        for chunk in bytes.chunks(4) {
            push_word(&mut out, *hash, big_endian);
            out.extend_from_slice(chunk);
        }
    }
    push_word(&mut out, 0xffff_ffff, big_endian);

    out
}

#[test]
fn cli_shows_what_the_save_select_screen_shows() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("save");
    make_save(&input, false);
    fs::create_dir_all(input.join("1")).unwrap();
    fs::write(
        input.join("1/game_data.sav"),
        record_table(V15, Platform::WiiU, &[(0xaaaa, 7)]),
    )
    .unwrap();

    // Slot 1 carries the manual-save flag and a place name; slot 0 has no
    // caption.sav at all, so its columns must stay empty instead of guessed.
    fs::write(
        input.join("1/caption.sav"),
        caption_table(
            Platform::WiiU,
            &[
                (0x0ba7_cf40, 1_792_679_016),
                (0xa067_8286, 8661),
                (0xe512_56e2, 0),
                (0x4acb_5004, 0),
            ],
            &[(0xf26f_1a02, "MapArea_HyruleForest", 24)],
        ),
    )
    .unwrap();

    let dry_run = ProcessCommand::new(env!("CARGO_BIN_EXE_botw-convert"))
        .arg("--dry-run")
        .arg(&input)
        .output()
        .unwrap();
    assert!(dry_run.status.success());
    let stdout = String::from_utf8_lossy(&dry_run.stdout);

    assert!(stdout.contains("normal manual"), "{stdout}");
    assert!(stdout.contains("2026-10-22 14:23"), "{stdout}");
    assert!(stdout.contains("144:21"), "{stdout}");
    assert!(stdout.contains("Hyrule Forest"), "{stdout}");
    assert!(
        stdout.contains("Saved"),
        "header should list the column: {stdout}"
    );
}
