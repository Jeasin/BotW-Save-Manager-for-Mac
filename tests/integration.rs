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

fn make_save(root: &Path, unknown_version: bool) -> Vec<u8> {
    fs::create_dir_all(root.join("0")).unwrap();

    fs::write(
        root.join("option.sav"),
        [0x00, 0x00, 0x47, 0x1b, 0x11, 0x22, 0x33, 0x44],
    )
    .unwrap();

    let header = if unknown_version {
        [0x00, 0x00, 0x00, 0x00]
    } else {
        [0x00, 0x00, 0x47, 0x1e]
    };
    let mut game_data = header.to_vec();
    game_data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);
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

/// A minimal Switch 2 Edition save: option.sav and one slot both use the
/// v1.7 header (0x4730).
fn make_switch_2_save(root: &Path) {
    fs::create_dir_all(root.join("7")).unwrap();

    fs::write(
        root.join("option.sav"),
        [
            0x30, 0x47, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x00, 0x00,
        ],
    )
    .unwrap();

    fs::write(
        root.join("7/game_data.sav"),
        [
            0x30, 0x47, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x00, 0x00,
        ],
    )
    .unwrap();
}

/// A Switch save that mixes a v1.5 slot with a v1.7 (Switch 2 Edition) one.
fn make_mixed_switch_save(root: &Path) {
    fs::create_dir_all(root.join("0")).unwrap();
    fs::create_dir_all(root.join("7")).unwrap();

    fs::write(
        root.join("option.sav"),
        [
            0x30, 0x47, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x00, 0x00,
        ],
    )
    .unwrap();

    fs::write(
        root.join("0/game_data.sav"),
        [0x1b, 0x47, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04],
    )
    .unwrap();

    fs::write(
        root.join("7/game_data.sav"),
        [
            0x30, 0x47, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x00, 0x00,
        ],
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
    assert_eq!(&converted[..4], &[0x1e, 0x47, 0x00, 0x00]);
    assert_eq!(&converted[4..8], &[0x04, 0x03, 0x02, 0x01]);

    let inspection = inspect_save(&output).unwrap();
    assert_eq!(inspection.platform, Platform::Switch);
    assert_eq!(inspection.version_summary(), "v1.6 x1");
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
    let already_switch = [0x1e, 0x47, 0x00, 0x00, 0x09, 0x08, 0x07, 0x06];
    fs::write(input.join("1/game_data.sav"), already_switch).unwrap();

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
        [0x1e, 0x47, 0x00, 0x00, 0x09, 0x08, 0x07, 0x06],
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
    assert_eq!(info.version_summary(), "v1.6 x2");
    assert!(info.file_breakdown().starts_with("2 slots"));
}

#[test]
fn refuses_switch_2_edition_saves_for_wii_u() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("switch2");
    let output = temporary.path().join("wiiu");
    make_switch_2_save(&input);

    let info = inspect_save(&input).unwrap();
    assert_eq!(info.platform, Platform::Switch);
    assert_eq!(info.version_summary(), "v1.7 Switch 2 Edition x1");

    let error = try_convert(&input, &output, &ConvertOptions::default()).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("v1.7"), "{message}");
    assert!(message.contains("option.sav"), "{message}");
    assert!(message.contains("--option-sav"), "{message}");
    assert!(message.contains("--drop-incompatible"), "{message}");
    assert!(!output.exists());

    // Known versions are not bypassed by --allow-unsupported.
    let options = ConvertOptions {
        allow_unsupported: true,
        ..ConvertOptions::default()
    };
    assert!(try_convert(&input, &output, &options).is_err());
    assert!(!output.exists());
}

#[test]
fn drops_incompatible_slots_when_given_a_wii_u_option_file() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("switch-mixed");
    let output = temporary.path().join("wiiu");
    let template = temporary.path().join("wiiu-template");
    make_mixed_switch_save(&input);

    // A known-good Wii U option.sav supplies the file the v1.7 save cannot.
    fs::create_dir_all(&template).unwrap();
    let wii_u_option = [0x00, 0x00, 0x47, 0x1b, 0x11, 0x22, 0x33, 0x44];
    fs::write(template.join("option.sav"), wii_u_option).unwrap();

    let options = ConvertOptions {
        drop_incompatible: true,
        option_sav: Some(template.join("option.sav")),
        ..ConvertOptions::default()
    };

    let info = inspect_save(&input).unwrap();
    assert_eq!(info.slots_to_drop(&options).len(), 1);

    let report = convert_save(&info, &output, &options).unwrap();

    assert_eq!(report.target, Platform::WiiU);
    assert!(report.option_replaced);
    assert_eq!(report.dropped, vec![PathBuf::from("7")]);
    assert!(output.join("option.sav").is_file());
    assert!(output.join("0/game_data.sav").is_file());
    assert!(!output.join("7").exists());

    assert_eq!(fs::read(output.join("option.sav")).unwrap(), wii_u_option);
    assert_eq!(
        fs::read(output.join("0/game_data.sav")).unwrap()[..4],
        Platform::WiiU.magic()
    );

    let converted = inspect_save(&output).unwrap();
    assert_eq!(converted.platform, Platform::WiiU);
    assert_eq!(converted.slots.len(), 1);
}

#[test]
fn rejects_an_option_file_from_the_wrong_platform() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("switch-mixed");
    let output = temporary.path().join("wiiu");
    make_mixed_switch_save(&input);

    let options = ConvertOptions {
        drop_incompatible: true,
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
    assert!(stdout.contains("v1.6 x1"), "{stdout}");
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
fn cli_exits_with_three_for_switch_2_edition() {
    let temporary = TestDirectory::new();
    let input = temporary.path().join("switch2");
    make_switch_2_save(&input);

    let blocked = ProcessCommand::new(env!("CARGO_BIN_EXE_botw-convert"))
        .arg("--dry-run")
        .arg(&input)
        .output()
        .unwrap();

    assert_eq!(blocked.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&blocked.stderr);
    assert!(stderr.contains("option.sav"), "{stderr}");
    assert!(stderr.contains("--drop-incompatible"), "{stderr}");
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
        [0x00, 0x00, 0x47, 0x1e, 0x01, 0x02, 0x03, 0x04],
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
