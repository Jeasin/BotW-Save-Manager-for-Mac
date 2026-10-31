// SPDX-License-Identifier: AGPL-3.0-only

use bsmm::caption::Caption;
use bsmm::cli::{Args, Command, HELP, parse_args};
use bsmm::error::{Error, Result};
use bsmm::save::{
    ConvertOptions, ConvertReport, SaveInfo, SaveSlot, SlotAction, convert_save,
    default_output_path, file_decisions, inspect_save, output_warning,
};
use std::env;
use std::path::Path;
use std::process;

fn main() {
    process::exit(run());
}

fn run() -> i32 {
    let command = match parse_args(env::args_os().skip(1)) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("Try 'bsmm --help' for usage.");
            return 2;
        }
    };

    let result = match command {
        Command::Help => {
            print!("{HELP}");
            Ok(())
        }
        Command::Version => {
            println!("bsmm {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Run(args) => run_command(&args),
    };

    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error}");
            exit_code(&error)
        }
    }
}

fn exit_code(error: &Error) -> i32 {
    match error {
        Error::UnsupportedSave(_) => 3,
        _ => 1,
    }
}

fn run_command(args: &Args) -> Result<()> {
    let options = ConvertOptions {
        allow_unsupported: args.allow_unsupported,
        drop_incompatible: args.drop_incompatible,
        option_sav: args.option_sav.clone(),
    };

    let info = inspect_save(&args.input)?;
    let requested = match &args.output {
        Some(path) => path.to_path_buf(),
        None => default_output_path(&info.root, info.target())?,
    };

    print_summary(&info, &requested);
    print_slots(&info, &options);
    if args.verbose {
        print_files(&info, &options)?;
    }
    print_warnings(&info, &options);

    let blockers = info.blockers(&options);
    if !blockers.is_empty() {
        return Err(Error::UnsupportedSave(blockers.join("\n  ")));
    }

    if args.dry_run {
        if let Some(warning) = output_warning(&info.root, &requested) {
            eprintln!("warning: {warning}");
        }
        println!("\nDry run: nothing was written.");
        return Ok(());
    }

    let report = convert_save(&info, &requested, &options)?;
    print_report(&report);
    Ok(())
}

fn print_summary(info: &SaveInfo, output: &Path) {
    println!("Source   : {}", info.root.display());
    println!("Save type: {} save", info.platform.display_label());
    println!(
        "option   : {} ({} bytes)",
        info.option_label(),
        info.option_size
    );
    println!("Versions : {}", info.version_summary());
    println!("Target   : {}", info.target().display_label());
    println!("Output   : {}", output.display());
    println!(
        "Files    : {} .sav ({})",
        info.sav_files.len(),
        info.file_breakdown()
    );
}

fn action_text(action: SlotAction, info: &SaveInfo) -> String {
    match action {
        SlotAction::Convert => format!("convert to {}", info.target().label()),
        SlotAction::ConvertDowngrade => "convert + downgrade".to_owned(),
        SlotAction::Copy => format!("keep, already {}", info.target().label()),
        SlotAction::KeepDowngrade => "keep + downgrade".to_owned(),
        SlotAction::Drop => "drop, unknown newer version".to_owned(),
        SlotAction::Blocked => "BLOCKED, needs --allow-unsupported".to_owned(),
    }
}

/// Longest region name printed in the slot table; longer names are shortened so
/// the row stays on one line.
const PLACE_WIDTH: usize = 16;

fn shorten(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }

    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn saved_text(slot: &SaveSlot) -> String {
    slot.caption
        .as_ref()
        .and_then(Caption::saved_label)
        .unwrap_or_else(|| "-".to_owned())
}

fn play_text(slot: &SaveSlot) -> String {
    slot.caption
        .as_ref()
        .and_then(Caption::play_label)
        .unwrap_or_else(|| "-".to_owned())
}

fn kind_text(slot: &SaveSlot) -> String {
    slot.caption
        .as_ref()
        .map_or("-", Caption::kind_label)
        .to_owned()
}

fn place_text(slot: &SaveSlot) -> String {
    slot.caption
        .as_ref()
        .and_then(Caption::location_label)
        .map(|text| shorten(&text, PLACE_WIDTH))
        .unwrap_or_else(|| "-".to_owned())
}

fn print_slots(info: &SaveInfo, options: &ConvertOptions) {
    if info.slots.is_empty() {
        return;
    }

    let dir_width = info
        .slots
        .iter()
        .map(|slot| slot.directory.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let version_width = info
        .slots
        .iter()
        .map(|slot| slot.version_label().len())
        .max()
        .unwrap_or(7)
        .max(7);
    // The save-select columns only make sense when the save ships caption.sav
    // files, which every real save does.
    let with_caption = info.slots.iter().any(|slot| slot.caption.is_some());
    let kind_width = info
        .slots
        .iter()
        .map(|slot| kind_text(slot).len())
        .max()
        .unwrap_or(1)
        .max(4);
    let place_width = info
        .slots
        .iter()
        .map(|slot| place_text(slot).chars().count())
        .max()
        .unwrap_or(1)
        .max(5);

    println!();
    if with_caption {
        println!(
            "{:<dir_width$}  {:<6}  {:<version_width$}  {:>9}  {:<16}  {:>7}  {:<kind_width$}  {:<place_width$}  Action",
            "Slot", "Stored", "Version", "Bytes", "Saved", "Play", "Kind", "Place"
        );
    } else {
        println!(
            "{:<dir_width$}  {:<6}  {:<version_width$}  {:>9}  Action",
            "Slot", "Stored", "Version", "Bytes"
        );
    }

    for slot in &info.slots {
        let action = info.slot_action(slot, options);
        if with_caption {
            println!(
                "{:<dir_width$}  {:<6}  {:<version_width$}  {:>9}  {:<16}  {:>7}  {:<kind_width$}  {:<place_width$}  {}",
                slot.directory,
                slot.platform_label(),
                slot.version_label(),
                slot.size,
                saved_text(slot),
                play_text(slot),
                kind_text(slot),
                place_text(slot),
                action_text(action, info)
            );
        } else {
            println!(
                "{:<dir_width$}  {:<6}  {:<version_width$}  {:>9}  {}",
                slot.directory,
                slot.platform_label(),
                slot.version_label(),
                slot.size,
                action_text(action, info)
            );
        }
    }

    if with_caption {
        println!();
        println!("Kind is read from each slot's caption.sav: normal mode has one manual save");
        println!(
            "and five autosaves, Master Mode one of each. Time is the moment the game wrote it."
        );
    }
}

fn print_files(info: &SaveInfo, options: &ConvertOptions) -> Result<()> {
    println!("\nFiles:");
    for (path, action) in file_decisions(info, options)? {
        let label = match action {
            SlotAction::Copy => "keep",
            SlotAction::KeepDowngrade => "keep, downgrade",
            SlotAction::ConvertDowngrade => "convert, downgrade",
            _ => "convert",
        };
        println!("  {:<52} {label}", path.display());
    }
    Ok(())
}

fn print_report(report: &ConvertReport) {
    println!();
    println!(
        "Done. Converted {} .sav to {} format.",
        report.files_converted,
        report.target.label()
    );
    if report.files_kept > 0 {
        println!(
            "      Kept {} .sav that were already {}.",
            report.files_kept,
            report.target.label()
        );
    }
    if report.files_downgraded > 0 {
        println!(
            "      Downgraded {} file(s) to the v1.5 layout ({} newer field(s) dropped).",
            report.files_downgraded, report.removed_fields
        );
    }
    if !report.dropped.is_empty() {
        println!(
            "      Left out {} slot(s) with no {} equivalent: {}",
            report.dropped.len(),
            report.target.label(),
            report
                .dropped
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if report.option_replaced {
        println!("      Replaced option.sav with the file given by --option-sav.");
    }
    println!("      {} files copied in total.", report.files_copied);
    println!("      Output: {}", report.output.display());
}

fn print_warnings(info: &SaveInfo, options: &ConvertOptions) {
    for warning in info.warnings(options) {
        eprintln!("warning: {warning}");
    }
}
