// SPDX-License-Identifier: AGPL-3.0-only

use botw_save_convert::cli::{Args, Command, HELP, parse_args};
use botw_save_convert::error::{Error, Result};
use botw_save_convert::save::{
    ConvertOptions, ConvertReport, SaveInfo, SlotAction, convert_save, default_output_path,
    file_decisions, inspect_save, output_warning,
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
            eprintln!("Try 'botw-convert --help' for usage.");
            return 2;
        }
    };

    let result = match command {
        Command::Help => {
            print!("{HELP}");
            Ok(())
        }
        Command::Version => {
            println!("botw-convert {}", env!("CARGO_PKG_VERSION"));
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
        SlotAction::ConvertDowngrade => {
            format!("convert to {}, drop newer fields", info.target().label())
        }
        SlotAction::Copy => format!("keep, already {}", info.target().label()),
        SlotAction::KeepDowngrade => "keep, drop newer fields".to_owned(),
        SlotAction::Drop => "drop, unknown newer version".to_owned(),
        SlotAction::Blocked => "BLOCKED, needs --allow-unsupported".to_owned(),
    }
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

    println!();
    println!(
        "{:<dir_width$}  {:<8}  {:<version_width$}  {:>9}  Action",
        "Slot", "Stored", "Version", "Bytes"
    );

    for slot in &info.slots {
        let action = info.slot_action(slot, options);
        println!(
            "{:<dir_width$}  {:<8}  {:<version_width$}  {:>9}  {}",
            slot.directory,
            slot.platform_label(),
            slot.version_label(),
            slot.size,
            action_text(action, info)
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
