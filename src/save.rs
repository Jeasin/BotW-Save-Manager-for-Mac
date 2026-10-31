// SPDX-License-Identifier: AGPL-3.0-only

use crate::convert::convert_sav_bytes;
use crate::error::{Error, Result};
use crate::platform::Platform;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

const OPTION_FILE: &str = "option.sav";
const GAME_DATA_FILE: &str = "game_data.sav";

#[derive(Clone, Debug)]
pub struct SaveSlot {
    pub relative_path: PathBuf,
    pub version: Option<&'static str>,
    pub raw_version: Option<u16>,
    /// Platform detected from this slot's own header, which can differ from the
    /// save as a whole when a folder was merged or partially converted.
    pub platform: Option<Platform>,
}

#[derive(Debug)]
pub struct SaveInfo {
    pub root: PathBuf,
    pub platform: Platform,
    pub option_version: Option<u16>,
    pub slots: Vec<SaveSlot>,
    pub sav_files: Vec<PathBuf>,
    pub unknown_versions: Vec<PathBuf>,
    pub foreign_slots: Vec<PathBuf>,
    pub versions: Vec<&'static str>,
}

impl SaveInfo {
    pub fn target(&self) -> Platform {
        self.platform.other()
    }

    pub fn has_unknown_versions(&self) -> bool {
        !self.unknown_versions.is_empty()
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

        for path in &self.foreign_slots {
            warnings.push(format!(
                "'{}' is already in {} format and will be copied without conversion",
                path.display(),
                self.platform.other().label()
            ));
        }

        if self.slots.is_empty() {
            warnings.push(
                "no game_data.sav was found; the game version could not be checked".to_owned(),
            );
        }

        warnings
    }

    /// Returns an explanation when the Wii U release cannot represent this
    /// save. The Wii U version of the game stopped at v1.6 (0x471e); Switch 2
    /// Edition saves (v1.7, 0x4730) have no Wii U equivalent.
    pub fn wii_u_blocker(&self) -> Option<String> {
        if self.platform == Platform::WiiU {
            return None;
        }

        let mut offenders: Vec<(String, u16)> = Vec::new();

        if let Some(raw) = self.option_version {
            if !Platform::is_wii_u_compatible(raw) {
                offenders.push((OPTION_FILE.to_owned(), raw));
            }
        }

        for slot in &self.slots {
            if slot.platform != Some(self.platform) {
                continue;
            }
            if let Some(raw) = slot.raw_version {
                if !Platform::is_wii_u_compatible(raw) {
                    offenders.push((slot.relative_path.display().to_string(), raw));
                }
            }
        }

        let (path, raw) = offenders.first()?;
        let name = Platform::version_name(*raw)
            .map(str::to_owned)
            .unwrap_or_else(|| "an unknown version".to_owned());

        Some(format!(
            "cannot convert this save to Wii U: '{path}' uses {name} (0x{raw:04x}), \
             but this converter only produces Wii U-compatible saves up to v1.6 (0x471e). \
             v1.7 and newer exist only on Switch / Switch 2 Edition and have no Wii U \
             equivalent, so a valid Wii U save cannot be produced. Use a v1.5/v1.6 \
             Switch save for the Switch -> Wii U direction."
        ))
    }
}

#[derive(Debug)]
pub struct ConvertReport {
    pub source: SaveInfo,
    pub target: Platform,
    pub files_copied: usize,
    pub sav_files_converted: usize,
    pub sav_files_skipped: usize,
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
    let option_version = platform.version_code(&option_header);

    let all_files = collect_relative_files(&root)?;
    let sav_files: Vec<PathBuf> = all_files
        .iter()
        .filter(|path| is_sav_file(path))
        .cloned()
        .collect();

    let mut slots = Vec::new();
    let mut unknown_versions = Vec::new();
    let mut foreign_slots = Vec::new();
    let mut versions = Vec::new();

    for relative_path in &all_files {
        if relative_path.file_name() != Some(OsStr::new(GAME_DATA_FILE)) {
            continue;
        }

        let path = root.join(relative_path);
        let header = read_header(&path)?;
        let slot_platform = header
            .as_ref()
            .and_then(|header| Platform::from_magic(header));
        let raw_version = match (slot_platform, header) {
            (Some(slot_platform), Some(header)) => slot_platform.version_code(&header),
            _ => None,
        };
        let version = raw_version.and_then(Platform::version_name);

        match slot_platform {
            Some(detected) if detected == platform => match version {
                Some(version) => {
                    if !versions.contains(&version) {
                        versions.push(version);
                    }
                }
                None => unknown_versions.push(relative_path.clone()),
            },
            Some(_) => foreign_slots.push(relative_path.clone()),
            None => unknown_versions.push(relative_path.clone()),
        }

        slots.push(SaveSlot {
            relative_path: relative_path.clone(),
            version,
            raw_version,
            platform: slot_platform,
        });
    }

    Ok(SaveInfo {
        root,
        platform,
        option_version,
        slots,
        sav_files,
        unknown_versions,
        foreign_slots,
        versions,
    })
}

/// Builds the implicit output folder used when the caller does not pass one:
/// a sibling of the input folder named `<input>-switch` or `<input>-wiiu`.
/// An existing platform suffix is replaced instead of appended, so converting
/// `save-wiiu` to Switch produces `save-switch`, not `save-wiiu-switch`.
pub fn default_output_path(input: &Path, target: Platform) -> Result<PathBuf> {
    let root =
        fs::canonicalize(input).map_err(|error| Error::io("open input folder", input, error))?;
    let parent = root
        .parent()
        .ok_or_else(|| Error::Output("the input folder has no parent directory".to_owned()))?;
    let name = root.file_name().ok_or_else(|| {
        Error::Output(format!(
            "'{}' does not end in a folder name",
            root.display()
        ))
    })?;

    let suffix = match target {
        Platform::Switch => "switch",
        Platform::WiiU => "wiiu",
    };

    let mut output_name = OsString::from(strip_platform_suffix(name));
    output_name.push("-");
    output_name.push(suffix);

    Ok(parent.join(output_name))
}

fn strip_platform_suffix(name: &OsStr) -> &OsStr {
    let Some(text) = name.to_str() else {
        return name;
    };
    let lower = text.to_ascii_lowercase();

    for suffix in ["-switch", "-wiiu"] {
        if lower.ends_with(suffix) {
            return OsStr::new(&text[..text.len() - suffix.len()]);
        }
    }

    name
}

/// Validates the destination and returns its absolute path.
///
/// The destination itself must not exist, which prevents accidental overwrites
/// and keeps the final rename atomic. Missing parent directories are created
/// when `create_parents` is true; `--dry-run` passes false so it stays
/// side-effect free.
pub fn validate_output_path(input: &Path, output: &Path, create_parents: bool) -> Result<PathBuf> {
    let input_canonical =
        fs::canonicalize(input).map_err(|error| Error::io("open input folder", input, error))?;
    let input_lexical = absolute_path(input)?;

    if output.as_os_str().is_empty() {
        return Err(Error::Output("the output path is empty".to_owned()));
    }

    let absolute = absolute_path(output)?;
    let name = absolute.file_name().ok_or_else(|| {
        Error::Output(format!(
            "'{}' is not a valid output directory name",
            output.display()
        ))
    })?;
    let parent = absolute
        .parent()
        .ok_or_else(|| Error::Output("the output path has no parent directory".to_owned()))?;

    if absolute.starts_with(&input_canonical) || absolute.starts_with(&input_lexical) {
        return Err(Error::Output(
            "the output folder cannot be inside the input folder".to_owned(),
        ));
    }

    match fs::symlink_metadata(&absolute) {
        Ok(_) => {
            return Err(Error::Output(format!(
                "'{}' already exists; choose a new output folder so nothing is overwritten",
                absolute.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::io("inspect output path", &absolute, error)),
    }

    match fs::metadata(parent) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(Error::Output(format!(
                "the output parent '{}' is not a directory",
                parent.display()
            )));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if !create_parents {
                return Ok(absolute);
            }
            fs::create_dir_all(parent)
                .map_err(|error| Error::io("create the output folder's parent", parent, error))?;
        }
        Err(error) => return Err(Error::io("inspect output parent", parent, error)),
    }

    let parent = fs::canonicalize(parent)
        .map_err(|error| Error::io("open the output folder's parent directory", parent, error))?;

    if !parent.is_dir() {
        return Err(Error::Output(format!(
            "the output parent '{}' is not a directory",
            parent.display()
        )));
    }

    let target = parent.join(name);
    if target.starts_with(&input_canonical) {
        return Err(Error::Output(
            "the output folder cannot be inside the input folder".to_owned(),
        ));
    }

    match fs::symlink_metadata(&target) {
        Ok(_) => Err(Error::Output(format!(
            "'{}' already exists; choose a new output folder so nothing is overwritten",
            target.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(target),
        Err(error) => Err(Error::io("inspect output path", &target, error)),
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| Error::io("read the current directory", Path::new("."), error))?
            .join(path)
    };

    Ok(normalize_lexically(&joined))
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => result.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            Component::Normal(part) => result.push(part),
        }
    }

    result
}

pub fn convert_save(
    input: &Path,
    output: Option<&Path>,
    allow_unsupported: bool,
) -> Result<ConvertReport> {
    let source = inspect_save(input)?;

    if source.has_unknown_versions() && !allow_unsupported {
        return Err(Error::UnsupportedSave(format!(
            "one or more game versions are unknown ({}); use --allow-unsupported to convert anyway",
            source.version_summary()
        )));
    }

    if source.target() == Platform::WiiU {
        if let Some(reason) = source.wii_u_blocker() {
            return Err(Error::UnsupportedSave(reason));
        }
    }

    let requested = match output {
        Some(path) => path.to_path_buf(),
        None => default_output_path(&source.root, source.target())?,
    };
    let output = validate_output_path(&source.root, &requested, true)?;
    let temporary = TemporaryDirectory::create(&output)?;
    let files_copied = copy_directory(&source.root, temporary.path())?;
    let mut sav_files_skipped = 0usize;

    for relative_path in &source.sav_files {
        let path = temporary.path().join(relative_path);
        let mut data =
            fs::read(&path).map_err(|error| Error::io("read save file", &path, error))?;
        let original_len = data.len();

        // A file that already carries the target platform's byte order was
        // converted earlier (or imported from the other platform). Swapping it
        // again would corrupt it, so copy it through untouched.
        if data.len() >= 4 && Platform::from_magic(&data[..4]) == Some(source.target()) {
            sav_files_skipped += 1;
            continue;
        }

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

    let sav_files_converted = source.sav_files.len() - sav_files_skipped;
    let target = source.target();
    temporary.persist(&output)?;

    Ok(ConvertReport {
        source,
        target,
        files_copied,
        sav_files_converted,
        sav_files_skipped,
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
