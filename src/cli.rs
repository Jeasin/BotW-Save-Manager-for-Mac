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
    "    -h, --help                  Print this help\n",
    "    -V, --version               Print the version\n",
    "\n",
    "Safety:\n",
    "    The input folder is never modified. An existing OUTPUT_DIR is never\n",
    "    overwritten. Missing parent directories of OUTPUT_DIR are created.\n",
    "    Conversion runs in a temporary folder and is moved into place only\n",
    "    after validation. Keep a separate backup of any important save.\n",
    "\n",
    "Limits:\n",
    "    Switch 2 Edition saves (v1.7, 0x4730) have no Wii U equivalent and\n",
    "    are rejected for the Switch -> Wii U direction.\n",
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Args {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub dry_run: bool,
    pub allow_unsupported: bool,
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
    let mut parse_options = true;

    for argument in arguments {
        if parse_options && argument == "--" {
            parse_options = false;
            continue;
        }

        if parse_options {
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

    #[test]
    fn parses_paths_and_options() {
        assert_eq!(
            parse_args(args(&["--dry-run", "in", "out"])).unwrap(),
            Command::Run(Args {
                input: PathBuf::from("in"),
                output: Some(PathBuf::from("out")),
                dry_run: true,
                allow_unsupported: false,
            })
        );
    }

    #[test]
    fn output_is_optional() {
        assert_eq!(
            parse_args(args(&["in"])).unwrap(),
            Command::Run(Args {
                input: PathBuf::from("in"),
                output: None,
                dry_run: false,
                allow_unsupported: false,
            })
        );
    }

    #[test]
    fn supports_option_terminator() {
        assert_eq!(
            parse_args(args(&["--", "-input", "-output"])).unwrap(),
            Command::Run(Args {
                input: PathBuf::from("-input"),
                output: Some(PathBuf::from("-output")),
                dry_run: false,
                allow_unsupported: false,
            })
        );
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
    }
}
