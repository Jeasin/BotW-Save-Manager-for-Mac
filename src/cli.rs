// SPDX-License-Identifier: AGPL-3.0-only

use std::ffi::OsString;
use std::path::PathBuf;

pub const HELP: &str = concat!(
    "botw-convert ",
    env!("CARGO_PKG_VERSION"),
    "\n",
    "Convert Breath of the Wild save folders between Wii U and Switch.\n",
    "\n",
    "Usage:\n",
    "    botw-convert [OPTIONS] <INPUT_DIR> [OUTPUT_DIR]\n",
    "\n",
    "Arguments:\n",
    "    <INPUT_DIR>     Folder containing option.sav and the save slots\n",
    "    [OUTPUT_DIR]    Folder to create for the converted save (must not exist).\n",
    "                    Omit it to create a sibling folder named\n",
    "                    <input>-switch or <input>-wiiu next to the input.\n",
    "\n",
    "Options:\n",
    "    -n, --dry-run               Inspect the save without writing anything\n",
    "        --allow-unsupported     Convert even if the game version is unknown\n",
    "        --drop-incompatible     Switch -> Wii U only: leave out slots this build\n",
    "                                has no field list for\n",
    "        --option-sav <FILE>     Switch -> Wii U only: use this Wii U option.sav\n",
    "                                in the output instead of converting the source one\n",
    "    -v, --verbose               List every .sav file that is processed\n",
    "    -h, --help                  Print this help\n",
    "    -V, --version               Print the version\n",
    "\n",
    "Exit codes:\n",
    "    0  success            1  failure            2  bad usage\n",
    "    3  unsupported save (unknown version, or a version this build has no\n",
    "       field list for)\n",
    "\n",
    "Safety:\n",
    "    The input folder is never modified. An existing OUTPUT_DIR is never\n",
    "    overwritten. Missing parent directories of OUTPUT_DIR are created.\n",
    "    Conversion runs in a temporary folder and is moved into place only\n",
    "    after validation. Keep a separate backup of any important save.\n",
    "\n",
    "Versions:\n",
    "    Wii U stops at v1.5 (0x471b). v1.6 (0x471e) and the Switch 2 Edition\n",
    "    format (0x4730) are byte-order variants of the same record table with a\n",
    "    few extra fields, so they are converted by dropping those extra fields\n",
    "    and retagging the header to v1.5. The fields that are dropped are new\n",
    "    settings this converter knows by hash; progress data is kept.\n",
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Args {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub dry_run: bool,
    pub allow_unsupported: bool,
    pub drop_incompatible: bool,
    pub option_sav: Option<PathBuf>,
    pub verbose: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Help,
    Version,
    Run(Args),
}

pub fn parse_args<I>(arguments: I) -> std::result::Result<Command, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut positional = Vec::new();
    let mut dry_run = false;
    let mut allow_unsupported = false;
    let mut drop_incompatible = false;
    let mut option_sav = None;
    let mut verbose = false;
    let mut parse_options = true;
    let mut arguments = arguments.into_iter();

    while let Some(argument) = arguments.next() {
        if parse_options && argument == "--" {
            parse_options = false;
            continue;
        }

        if parse_options {
            if let Some(text) = argument.to_str() {
                if let Some(value) = text.strip_prefix("--option-sav=") {
                    if value.is_empty() {
                        return Err("--option-sav needs a file path".to_owned());
                    }
                    option_sav = Some(PathBuf::from(value));
                    continue;
                }
            }

            match argument.to_str() {
                Some("-h" | "--help") => return Ok(Command::Help),
                Some("-V" | "--version") => return Ok(Command::Version),
                Some("-n" | "--dry-run") => {
                    dry_run = true;
                    continue;
                }
                Some("--allow-unsupported") => {
                    allow_unsupported = true;
                    continue;
                }
                Some("--drop-incompatible") => {
                    drop_incompatible = true;
                    continue;
                }
                Some("-v" | "--verbose") => {
                    verbose = true;
                    continue;
                }
                Some("--option-sav") => {
                    let value = arguments
                        .next()
                        .ok_or_else(|| "--option-sav needs a file path".to_owned())?;
                    option_sav = Some(PathBuf::from(value));
                    continue;
                }
                Some(value) if value.starts_with('-') => {
                    return Err(format!("unknown option '{value}'"));
                }
                _ => {}
            }
        }

        positional.push(argument);
    }

    if positional.is_empty() || positional.len() > 2 {
        return Err(format!(
            "expected 1 or 2 paths (input and optional output), got {}",
            positional.len()
        ));
    }

    let input = PathBuf::from(positional.remove(0));
    let output = positional.pop().map(PathBuf::from);

    Ok(Command::Run(Args {
        input,
        output,
        dry_run,
        allow_unsupported,
        drop_incompatible,
        option_sav,
        verbose,
    }))
}

#[cfg(test)]
mod tests {
    use super::{Args, Command, parse_args};
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    fn run(values: &[&str]) -> Args {
        match parse_args(args(values)).unwrap() {
            Command::Run(args) => args,
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parses_paths_and_options() {
        let parsed = run(&["--dry-run", "in", "out"]);
        assert_eq!(parsed.input, PathBuf::from("in"));
        assert_eq!(parsed.output, Some(PathBuf::from("out")));
        assert!(parsed.dry_run);
        assert!(!parsed.allow_unsupported);
    }

    #[test]
    fn output_is_optional() {
        let parsed = run(&["in"]);
        assert_eq!(parsed.input, PathBuf::from("in"));
        assert_eq!(parsed.output, None);
    }

    #[test]
    fn parses_wii_u_specific_flags() {
        let parsed = run(&["--drop-incompatible", "--option-sav", "opt.sav", "in"]);
        assert!(parsed.drop_incompatible);
        assert_eq!(parsed.option_sav, Some(PathBuf::from("opt.sav")));

        let parsed = run(&["--option-sav=opt.sav", "in", "out"]);
        assert_eq!(parsed.option_sav, Some(PathBuf::from("opt.sav")));
    }

    #[test]
    fn supports_option_terminator() {
        let parsed = run(&["--", "-input", "-output"]);
        assert_eq!(parsed.input, PathBuf::from("-input"));
        assert_eq!(parsed.output, Some(PathBuf::from("-output")));
    }

    #[test]
    fn parses_help_and_version() {
        assert_eq!(parse_args(args(&["--help"])).unwrap(), Command::Help);
        assert_eq!(parse_args(args(&["-V"])).unwrap(), Command::Version);
    }

    #[test]
    fn rejects_bad_arguments() {
        assert!(parse_args(args(&["--wat", "in", "out"])).is_err());
        assert!(parse_args(args(&[])).is_err());
        assert!(parse_args(args(&["in", "out", "extra"])).is_err());
        assert!(parse_args(args(&["in", "--option-sav"])).is_err());
    }
}
