// SPDX-License-Identifier: AGPL-3.0-only

use crate::convert::convert_sav_bytes;
use crate::error::{Error, Result};
use crate::platform::Platform;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

const OPTION_FILE: &str = "option.sav";
const GAME_DATA_FILE: &str = "game_data.sav";

#[derive(Clone, Debug)]
pub struct SaveSlot {
    pub relative_path: PathBuf,
    pub version: Option<&'static str>,
}

#[derive(Debug)]
pub struct SaveInfo {
    pub root: PathBuf,
    pub platform: Platform,
    pub slots: Vec<SaveSlot>,
    pub sav_files: Vec<PathBuf>,
    pub unknown_versions: Vec<PathBuf>,
    pub versions: Vec<&'static str>,
}

impl SaveInfo {
    pub fn target(&self) -> Platform {
        self.platform.other()
    }

    pub fn has_unsupported_versions(&self) -> bool {
        !self.unknown_versions.is_empty() || self.is_mixed_version()
    }

    pub fn is_mixed_version(&self) -> bool {
        self.versions.len() > 1
    }

    pub fn version_summary(&self) -> String {
        if self.slots.is_empty() {
            "no game_data.sav".to_owned()
        } else if self.versions.is_empty() {
            "unknown".to_owned()
        } else {
            self.versions.join(", ")
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        for path in &self.unknown_versions {
            warnings.push(format!(
                "could not identify the game version of '{}'",
                path.display()
            ));
        }

        if self.is_mixed_version() {
            warnings.push(format!(
                "save slots use different game versions: {}",
                self.versions.join(", ")
            ));
        }

        if self.slots.is_empty() {
            warnings.push(
                "no game_data.sav was found; the game version could not be checked".to_owned(),
            );
        }

        warnings
    }
}

#[derive(Debug)]
pub struct ConvertReport {
    pub source: SaveInfo,
    pub target: Platform,
    pub files_copied: usize,
    pub sav_files_converted: usize,
    pub output: PathBuf,
}

pub fn inspect_save(input: &Path) -> Result<SaveInfo> {
    let root =
        fs::canonicalize(input).map_err(|error| Error::io("open input folder", input, error))?;
    let metadata =
        fs::metadata(&root).map_err(|error| Error::io("inspect input folder", &root, error))?;

    if !metadata.is_dir() {
        return Err(Error::InvalidInput(format!(
            "'{}' is not a directory",
            root.display()
        )));
    }

    let option_path = root.join(OPTION_FILE);
    if !option_path.is_file() {
        return Err(Error::InvalidInput(format!(
            "'{}' does not contain option.sav at its root",
            root.display()
        )));
    }

    let option_header = read_header(&option_path)?.ok_or_else(|| {
        Error::InvalidInput(format!(
            "'{}' is shorter than 4 bytes",
            option_path.display()
        ))
    })?;
    let platform = Platform::from_magic(&option_header).ok_or_else(|| {
        Error::UnsupportedSave(format!(
            "'{}' is not a recognized Breath of the Wild save (unknown option.sav magic)",
            option_path.display()
        ))
    })?;

    let all_files = collect_relative_files(&root)?;
    let sav_files: Vec<PathBuf> = all_files
        .iter()
        .filter(|path| is_sav_file(path))
        .cloned()
        .collect();

    let mut slots = Vec::new();
    let mut unknown_versions = Vec::new();
    let mut versions = Vec::new();

    for relative_path in &all_files {
        if relative_path.file_name() != Some(OsStr::new(GAME_DATA_FILE)) {
            continue;
        }

        let path = root.join(relative_path);
        let version = match read_header(&path)? {
            Some(header) => platform.version_from_header(&header),
            None => None,
        };

        match version {
            Some(version) => {
                if !versions.contains(&version) {
                    versions.push(version);
                }
            }
            None => unknown_versions.push(relative_path.clone()),
        }

        slots.push(SaveSlot {
            relative_path: relative_path.clone(),
            version,
        });
    }

    Ok(SaveInfo {
        root,
        platform,
        slots,
        sav_files,
        unknown_versions,
        versions,
    })
}

/// Validates the destination and returns its absolute, canonicalized parent
/// joined with the requested folder name. The destination itself must not
/// exist, which prevents accidental overwrites and makes the final rename
/// atomic.
pub fn validate_output_path(input: &Path, output: &Path) -> Result<PathBuf> {
    let input =
        fs::canonicalize(input).map_err(|error| Error::io("open input folder", input, error))?;

    if output.as_os_str().is_empty() {
        return Err(Error::Output("the output path is empty".to_owned()));
    }

    let name = output.file_name().ok_or_else(|| {
        Error::Output(format!(
            "'{}' is not a valid output directory name",
            output.display()
        ))
    })?;

    let parent = match output.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let parent = fs::canonicalize(parent).map_err(|error| {
        Error::io(
            "open the output folder's parent directory",
            output.parent().unwrap_or(output),
            error,
        )
    })?;

    if !parent.is_dir() {
        return Err(Error::Output(format!(
            "the output parent '{}' is not a directory",
            parent.display()
        )));
    }

    let target = parent.join(name);
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            return Err(Error::Output(format!(
                "'{}' already exists; choose a new output folder so nothing is overwritten",
                target.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::io("inspect output path", &target, error)),
    }

    if target.starts_with(&input) {
        return Err(Error::Output(
            "the output folder cannot be inside the input folder".to_owned(),
        ));
    }

    Ok(target)
}

pub fn convert_save(input: &Path, output: &Path, allow_unsupported: bool) -> Result<ConvertReport> {
    let source = inspect_save(input)?;

    if source.has_unsupported_versions() && !allow_unsupported {
        return Err(Error::UnsupportedSave(format!(
            "one or more game versions are unknown or mixed ({}); use --allow-unsupported to convert anyway",
            source.version_summary()
        )));
    }

    let output = validate_output_path(&source.root, output)?;
    let temporary = TemporaryDirectory::create(&output)?;
    let files_copied = copy_directory(&source.root, temporary.path())?;

    for relative_path in &source.sav_files {
        let path = temporary.path().join(relative_path);
        let mut data =
            fs::read(&path).map_err(|error| Error::io("read save file", &path, error))?;
        let original_len = data.len();
        let is_trackblock = path.to_string_lossy().contains("trackblock");

        convert_sav_bytes(&mut data, is_trackblock);

        if data.len() != original_len {
            return Err(Error::Verification(format!(
                "the size of '{}' changed during conversion",
                relative_path.display()
            )));
        }

        fs::write(&path, &data).map_err(|error| Error::io("write converted save", &path, error))?;
    }

    verify_converted_save(temporary.path(), &source, source.target())?;

    let sav_files_converted = source.sav_files.len();
    let target = source.target();
    temporary.persist(&output)?;

    Ok(ConvertReport {
        source,
        target,
        files_copied,
        sav_files_converted,
        output,
    })
}

fn verify_converted_save(output: &Path, source: &SaveInfo, target: Platform) -> Result<()> {
    let converted = inspect_save(output).map_err(|error| {
        Error::Verification(format!("could not read the converted output: {error}"))
    })?;

    if converted.platform != target {
        return Err(Error::Verification(format!(
            "option.sav identifies as {}, expected {}",
            converted.platform.label(),
            target.label()
        )));
    }

    if converted.sav_files.len() != source.sav_files.len() {
        return Err(Error::Verification(
            "the number of .sav files changed during conversion".to_owned(),
        ));
    }

    for source_slot in &source.slots {
        let Some(expected_version) = source_slot.version else {
            continue;
        };

        let converted_slot = converted
            .slots
            .iter()
            .find(|slot| slot.relative_path == source_slot.relative_path)
            .ok_or_else(|| {
                Error::Verification(format!(
                    "missing converted slot '{}'",
                    source_slot.relative_path.display()
                ))
            })?;

        if converted_slot.version != Some(expected_version) {
            return Err(Error::Verification(format!(
                "game version changed in '{}' (was {expected_version}, now {})",
                source_slot.relative_path.display(),
                converted_slot.version.unwrap_or("unknown")
            )));
        }
    }

    Ok(())
}

fn is_sav_file(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("sav"))
}

fn read_header(path: &Path) -> Result<Option<[u8; 4]>> {
    let mut file = File::open(path).map_err(|error| Error::io("open file", path, error))?;
    let mut header = [0u8; 4];

    match file.read_exact(&mut header) {
        Ok(()) => Ok(Some(header)),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(Error::io("read file header", path, error)),
    }
}

fn collect_relative_files(root: &Path) -> Result<Vec<PathBuf>> {
    fn visit(base: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        let mut entries = fs::read_dir(directory)
            .map_err(|error| Error::io("read directory", directory, error))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| Error::io("read directory entry", directory, error))?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| Error::io("inspect directory entry", &path, error))?;

            if file_type.is_symlink() {
                return Err(Error::InvalidInput(format!(
                    "symbolic links are not supported: '{}'",
                    path.display()
                )));
            }

            if file_type.is_dir() {
                visit(base, &path, files)?;
            } else if file_type.is_file() {
                let relative = path.strip_prefix(base).map_err(|_| {
                    Error::InvalidInput(format!(
                        "could not make '{}' relative to '{}'",
                        path.display(),
                        base.display()
                    ))
                })?;
                files.push(relative.to_path_buf());
            } else {
                return Err(Error::InvalidInput(format!(
                    "unsupported filesystem entry: '{}'",
                    path.display()
                )));
            }
        }

        Ok(())
    }

    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn copy_directory(source: &Path, destination: &Path) -> Result<usize> {
    fn visit(source: &Path, destination: &Path, copied: &mut usize) -> Result<()> {
        let mut entries = fs::read_dir(source)
            .map_err(|error| Error::io("read source directory", source, error))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| Error::io("read source directory entry", source, error))?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let file_type = entry.file_type().map_err(|error| {
                Error::io("inspect source directory entry", &source_path, error)
            })?;

            if file_type.is_symlink() {
                return Err(Error::InvalidInput(format!(
                    "symbolic links are not supported: '{}'",
                    source_path.display()
                )));
            }

            if file_type.is_dir() {
                fs::create_dir(&destination_path).map_err(|error| {
                    Error::io("create output directory", &destination_path, error)
                })?;
                visit(&source_path, &destination_path, copied)?;
            } else if file_type.is_file() {
                fs::copy(&source_path, &destination_path)
                    .map_err(|error| Error::io("copy save file", &source_path, error))?;
                *copied += 1;
            } else {
                return Err(Error::InvalidInput(format!(
                    "unsupported filesystem entry: '{}'",
                    source_path.display()
                )));
            }
        }

        Ok(())
    }

    let mut copied = 0;
    visit(source, destination, &mut copied)?;
    Ok(copied)
}

struct TemporaryDirectory {
    path: PathBuf,
    armed: bool,
}

impl TemporaryDirectory {
    fn create(target: &Path) -> Result<Self> {
        let parent = target
            .parent()
            .ok_or_else(|| Error::Output("the output path has no parent directory".to_owned()))?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        for attempt in 0..100 {
            let path = parent.join(format!(".botw-convert-{}-{stamp}-{attempt}", process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self { path, armed: true });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(Error::io("create temporary output directory", path, error));
                }
            }
        }

        Err(Error::Output(
            "could not create a unique temporary output directory".to_owned(),
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn persist(mut self, target: &Path) -> Result<()> {
        match fs::symlink_metadata(target) {
            Ok(_) => {
                return Err(Error::Output(format!(
                    "'{}' appeared while converting; refusing to overwrite it",
                    target.display()
                )));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::io("inspect output path", target, error)),
        }

        fs::rename(&self.path, target)
            .map_err(|error| Error::io("move converted save into place", target, error))?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
