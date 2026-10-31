// SPDX-License-Identifier: AGPL-3.0-only

use botw_save_convert::cli::{Command, HELP, parse_args};
use botw_save_convert::error::Result;
use botw_save_convert::save::{
    ConvertReport, SaveInfo, convert_save, default_output_path, inspect_save, validate_output_path,
};
use std::env;
use std::path::{Path, PathBuf};
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
        Command::Run(args) if args.dry_run => run_dry(&args.input, args.output.as_deref()),
        Command::Run(args) => {
            run_conversion(&args.input, args.output.as_deref(), args.allow_unsupported)
        }
    };

    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

fn resolve_output(info: &SaveInfo, requested: Option<&Path>) -> Result<PathBuf> {
    match requested {
        Some(path) => Ok(path.to_path_buf()),
        None => default_output_path(&info.root, info.target()),
    }
}

fn run_dry(input: &Path, output: Option<&Path>) -> Result<()> {
    let info = inspect_save(input)?;
    let requested = resolve_output(&info, output)?;
    print_summary(&info, &requested);
    print_warnings(&info);

    if let Some(reason) = info.wii_u_blocker() {
        eprintln!("note: {reason}");
    } else if info.has_unknown_versions() {
        eprintln!(
            "note: conversion requires --allow-unsupported because the game version is unknown"
        );
    }

    validate_output_path(&info.root, &requested, false)?;

    println!("\nDry run: nothing was written.");
    Ok(())
}

fn run_conversion(input: &Path, output: Option<&Path>, allow_unsupported: bool) -> Result<()> {
    let report = convert_save(input, output, allow_unsupported)?;
    print_report(&report);
    Ok(())
}

fn print_summary(info: &SaveInfo, output: &Path) {
    println!(
        "Detected : {} save ({})",
        info.platform.display_label(),
        info.version_summary()
    );
    println!("Target   : {}", info.target().display_label());
    println!("Files    : {} .sav", info.sav_files.len());
    println!("Output   : {}", output.display());
}

fn print_report(report: &ConvertReport) {
    print_summary(&report.source, &report.output);
    print_warnings(&report.source);
    if report.sav_files_skipped > 0 {
        println!(
            "Skipped  : {} .sav already in {} format (copied unchanged)",
            report.sav_files_skipped,
            report.target.label()
        );
    }
    println!(
        "\nDone. Converted {} .sav files ({} files copied total) to {} format.",
        report.sav_files_converted,
        report.files_copied,
        report.target.label()
    );
}

fn print_warnings(info: &SaveInfo) {
    for warning in info.warnings() {
        eprintln!("warning: {warning}");
    }
}
