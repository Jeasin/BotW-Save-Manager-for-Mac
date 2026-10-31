// SPDX-License-Identifier: AGPL-3.0-only

use crate::caption::Caption;
use crate::convert::convert_sav_bytes;
use crate::error::{Error, Result};
use crate::platform::{Platform, WII_U_SAFE_VERSION};
use crate::records::{self, Kind};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

const OPTION_FILE: &str = "option.sav";
const GAME_DATA_FILE: &str = "game_data.sav";
const CAPTION_FILE: &str = "caption.sav";

/// What the converter will do with one save slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlotAction {
    /// Byte order is swapped into the target platform.
    Convert,
    /// Byte order is swapped and the newer fields are dropped so the Wii U
    /// release sees a v1.5 record set.
    ConvertDowngrade,
    /// Slot already uses the target platform's byte order; copied untouched.
    Copy,
    /// Slot already uses the target platform's byte order but carries newer
    /// fields, which are dropped.
    KeepDowngrade,
    /// Slot cannot exist on the target platform and will be left out.
    Drop,
    /// Slot cannot exist on the target platform and no flag allows dropping it.
    Blocked,
}

#[derive(Clone, Debug)]
pub struct SaveSlot {
    pub relative_path: PathBuf,
    /// Slot folder name, e.g. `0` or `7`.
    pub directory: String,
    pub size: u64,
    /// Platform detected from this slot's own header, which can differ from the
    /// save as a whole when a folder was merged or partially converted.
    pub platform: Option<Platform>,
    pub raw_version: Option<u16>,
    /// What the save-select screen shows for this slot, read from the
    /// `caption.sav` beside it. `None` when that file is absent or unreadable;
    /// it never affects the conversion itself.
    pub caption: Option<Caption>,
}

impl SaveSlot {
    pub fn version_label(&self) -> String {
        match self.raw_version {
            Some(raw) => Platform::version_label(raw),
            None => "unknown".to_owned(),
        }
    }

    pub fn platform_label(&self) -> &'static str {
        match self.platform {
            Some(Platform::WiiU) => "Wii U",
            Some(Platform::Switch) => "Switch",
            None => "-",
        }
    }
}

#[derive(Debug)]
pub struct SaveInfo {
    pub root: PathBuf,
    pub platform: Platform,
    pub option_size: u64,
    pub option_version: Option<u16>,
    pub slots: Vec<SaveSlot>,
    pub sav_files: Vec<PathBuf>,
    pub unknown_versions: Vec<PathBuf>,
    /// Slots stored in the *other* platform's byte order than the save itself.
    pub foreign_slots: Vec<PathBuf>,
    /// File-manager / cloud-sync duplicate copies such as `game_data-NSConflict-5.sav`.
    pub duplicates: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct ConvertOptions {
    pub allow_unsupported: bool,
    /// Keep slots the target platform cannot represent, dropping them instead of
    /// refusing the whole conversion.
    pub drop_incompatible: bool,
    /// Replace the output `option.sav` with this file (Wii U target only).
    pub option_sav: Option<PathBuf>,
}

impl SaveInfo {
    pub fn target(&self) -> Platform {
        self.platform.other()
    }

    pub fn has_unknown_versions(&self) -> bool {
        !self.unknown_versions.is_empty()
    }

    pub fn option_label(&self) -> String {
        match self.option_version {
            Some(raw) => Platform::version_label(raw),
            None => "unknown".to_owned(),
        }
    }

    /// `v1.5 x4, v1.6 x2, v1.8/v1.9 x2`
    pub fn version_summary(&self) -> String {
        if self.slots.is_empty() {
            return "no game_data.sav".to_owned();
        }

        let mut counts: Vec<(String, usize)> = Vec::new();
        for slot in &self.slots {
            let label = slot.version_label();
            match counts.iter_mut().find(|(name, _)| *name == label) {
                Some((_, count)) => *count += 1,
                None => counts.push((label, 1)),
            }
        }

        counts
            .into_iter()
            .map(|(name, count)| format!("{name} x{count}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// File categories, used for the `Files:` line so the totals are not opaque.
    pub fn file_breakdown(&self) -> String {
        let mut duplicates = 0;
        let mut slots = 0;
        let mut captions = 0;
        let mut trackers = 0;
        let mut option = 0;
        let mut other = 0;

        for path in &self.sav_files {
            let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
            if is_conflict_copy(name) {
                duplicates += 1;
            } else if name == OPTION_FILE {
                option += 1;
            } else if name == GAME_DATA_FILE {
                slots += 1;
            } else if name.starts_with("caption") {
                captions += 1;
            } else if name.starts_with("trackblock") {
                trackers += 1;
            } else {
                other += 1;
            }
        }

        let mut parts = vec![format!("{slots} slots"), format!("{trackers} trackers")];
        if captions > 0 {
            parts.push(format!("{captions} captions"));
        }
        if option > 0 {
            parts.push(format!("{option} option"));
        }
        if other > 0 {
            parts.push(format!("{other} other"));
        }
        if duplicates > 0 {
            parts.push(format!("{duplicates} duplicate copies"));
        }
        parts.join(", ")
    }

    pub fn slot_action(&self, slot: &SaveSlot, options: &ConvertOptions) -> SlotAction {
        let target = self.target();
        let downgrade =
            target == Platform::WiiU && slot.raw_version.is_some_and(Platform::needs_downgrade);

        if slot.platform == Some(target) {
            return if downgrade {
                SlotAction::KeepDowngrade
            } else {
                SlotAction::Copy
            };
        }

        if downgrade {
            // A version newer than the field lists this build knows cannot be
            // downgraded exactly; the caller has to accept that explicitly.
            let known = slot.raw_version.is_some_and(Platform::has_known_layout);
            if !known {
                if options.drop_incompatible {
                    return SlotAction::Drop;
                }
                if !options.allow_unsupported {
                    return SlotAction::Blocked;
                }
            }
            return SlotAction::ConvertDowngrade;
        }

        SlotAction::Convert
    }

    /// Slots that will be left out of the output.
    pub fn slots_to_drop(&self, options: &ConvertOptions) -> Vec<&SaveSlot> {
        self.slots
            .iter()
            .filter(|slot| self.slot_action(slot, options) == SlotAction::Drop)
            .collect()
    }

    /// Everything that prevents conversion, one line per offending file.
    pub fn blockers(&self, options: &ConvertOptions) -> Vec<String> {
        let mut blockers = Vec::new();

        if self.has_unknown_versions() && !options.allow_unsupported {
            for path in &self.unknown_versions {
                blockers.push(format!(
                    "'{}' has an unrecognised game version (use --allow-unsupported to convert anyway)",
                    path.display()
                ));
            }
        }

        if self.target() != Platform::WiiU {
            return blockers;
        }

        let option_allowed = match (self.option_version, &options.option_sav) {
            // A replacement option.sav is supplied, so the source one is unused.
            (_, Some(_)) => true,
            (Some(raw), None) => {
                Platform::is_wii_u_safe(raw)
                    || (Platform::needs_downgrade(raw)
                        && (Platform::has_known_layout(raw) || options.allow_unsupported))
            }
            (None, None) => true,
        };

        if !option_allowed {
            blockers.push(format!(
                "'{OPTION_FILE}' is {} and this build has no field list for that version; \
                 add --allow-unsupported to downgrade it with the fields it knows, or \
                 pass --option-sav <wiiu-option.sav> to supply a compatible one",
                self.option_label()
            ));
        }

        for slot in &self.slots {
            if self.slot_action(slot, options) != SlotAction::Blocked {
                continue;
            }

            blockers.push(format!(
                "'{}' is {} and this build has no field list for that version; add \
                 --allow-unsupported to downgrade it with the fields it knows, or \
                 --drop-incompatible to leave this slot out",
                slot.relative_path.display(),
                slot.version_label()
            ));
        }

        blockers
    }

    pub fn warnings(&self, options: &ConvertOptions) -> Vec<String> {
        let mut warnings = Vec::new();

        for path in &self.unknown_versions {
            if options.allow_unsupported {
                warnings.push(format!(
                    "'{}' has an unknown game version and will be byte-swapped anyway",
                    path.display()
                ));
            }
        }

        if self.foreign_slots.len() < 4 && !self.foreign_slots.is_empty() {
            for path in &self.foreign_slots {
                warnings.push(format!(
                    "'{}' already uses {} byte order inside a {} save, most likely left \
                     over from an earlier partial conversion; it is copied unchanged so it \
                     is not swapped twice",
                    path.display(),
                    self.target().label(),
                    self.platform.label()
                ));
            }
        } else if !self.foreign_slots.is_empty() {
            warnings.push(format!(
                "{} slots already use {} byte order; they are copied unchanged",
                self.foreign_slots.len(),
                self.target().label()
            ));
        }

        if self.target() == Platform::WiiU {
            let newer: Vec<&SaveSlot> = self
                .slots
                .iter()
                .filter(|slot| slot.raw_version.is_some_and(Platform::needs_downgrade))
                .collect();

            if !newer.is_empty() {
                warnings.push(format!(
                    "{} slot(s) use a save format newer than the Wii U release ({}); their \
                     extra fields are dropped and the header is rewritten to v1.5, so the \
                     Wii U game sees the record set it knows",
                    newer.len(),
                    newer
                        .iter()
                        .map(|slot| format!("{} {}", slot.directory, slot.version_label()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }

            if self.option_version.is_some_and(Platform::needs_downgrade)
                && options.option_sav.is_none()
            {
                warnings.push(format!(
                    "{OPTION_FILE} is {}; its newer fields are dropped to restore the \
                     336-byte v1.5 layout the Wii U release writes",
                    self.option_label()
                ));
            }

            let unknown: Vec<&SaveSlot> = self
                .slots
                .iter()
                .filter(|slot| {
                    slot.raw_version.is_some_and(|raw| {
                        Platform::needs_downgrade(raw) && !Platform::has_known_layout(raw)
                    })
                })
                .collect();

            if !unknown.is_empty() && !options.drop_incompatible {
                warnings.push(format!(
                    "{} slot(s) use a version this build has no field list for ({}); only \
                     the newer fields with known hashes are dropped, so fields this build \
                     does not know may survive - check those slots in game",
                    unknown.len(),
                    unknown
                        .iter()
                        .map(|slot| format!("{} {}", slot.directory, slot.version_label()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }

            let dropped = self.slots_to_drop(options);
            if !dropped.is_empty() {
                warnings.push(format!(
                    "{} slot(s) will be left out of the Wii U output: {}",
                    dropped.len(),
                    dropped
                        .iter()
                        .map(|slot| format!("{} ({})", slot.directory, slot.version_label()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }

        if !self.duplicates.is_empty() {
            warnings.push(format!(
                "{} duplicate copy/copies found ({}); they are treated like normal .sav files",
                self.duplicates.len(),
                self.duplicates
                    .iter()
                    .take(3)
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
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
    pub target: Platform,
    pub files_copied: usize,
    pub files_converted: usize,
    pub files_kept: usize,
    /// Files whose header and record set were rewritten to the v1.5 layout.
    pub files_downgraded: usize,
    /// Records removed while downgrading (newer fields the Wii U release has
    /// no slot for).
    pub removed_fields: usize,
    pub dropped: Vec<PathBuf>,
    pub option_replaced: bool,
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
    let option_size = fs::metadata(&option_path)
        .map_err(|error| Error::io("inspect option.sav", &option_path, error))?
        .len();

    let all_files = collect_relative_files(&root)?;
    let sav_files: Vec<PathBuf> = all_files
        .iter()
        .filter(|path| is_sav_file(path))
        .cloned()
        .collect();
    let mut duplicates = Vec::new();
    let mut slots = Vec::new();
    let mut unknown_versions = Vec::new();
    let mut foreign_slots = Vec::new();

    for relative_path in &all_files {
        if let Some(name) = relative_path.file_name().and_then(OsStr::to_str) {
            if is_sav_file(relative_path) && is_conflict_copy(name) {
                duplicates.push(relative_path.clone());
            }
        }

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
        let size = fs::metadata(&path)
            .map_err(|error| Error::io("inspect save slot", &path, error))?
            .len();
        let caption = fs::read(path.with_file_name(CAPTION_FILE))
            .ok()
            .and_then(|bytes| Caption::parse(&bytes));

        match slot_platform {
            Some(detected) if detected != platform => foreign_slots.push(relative_path.clone()),
            None => unknown_versions.push(relative_path.clone()),
            Some(_) => {}
        }

        if slot_platform.is_some() && raw_version.is_none() {
            unknown_versions.push(relative_path.clone());
        }

        slots.push(SaveSlot {
            relative_path: relative_path.clone(),
            directory: relative_path
                .parent()
                .and_then(Path::to_str)
                .unwrap_or("?")
                .to_owned(),
            size,
            platform: slot_platform,
            raw_version,
            caption,
        });
    }

    Ok(SaveInfo {
        root,
        platform,
        option_size,
        option_version,
        slots,
        sav_files,
        unknown_versions,
        foreign_slots,
        duplicates,
    })
}

/// Per-file decision list used by `--verbose`. Reads the first four bytes of
/// every `.sav` so files that already use the target byte order are reported
/// instead of being silently swapped twice.
pub fn file_decisions(
    info: &SaveInfo,
    options: &ConvertOptions,
) -> Result<Vec<(PathBuf, SlotAction)>> {
    let dropped = info.slots_to_drop(options);
    let mut decisions = Vec::with_capacity(info.sav_files.len());

    for relative in &info.sav_files {
        if dropped
            .iter()
            .any(|slot| relative.starts_with(&slot.directory))
        {
            continue;
        }

        let path = info.root.join(relative);
        let header = read_header(&path)?;
        let detected = header
            .as_ref()
            .and_then(|header| Platform::from_magic(header));
        let version = match (detected, &header) {
            (Some(platform), Some(header)) => platform.version_code(header),
            _ => None,
        };
        let downgrade = info.target() == Platform::WiiU
            && version.is_some_and(Platform::needs_downgrade)
            && relative
                .file_name()
                .and_then(OsStr::to_str)
                .and_then(Kind::from_name)
                .is_some();

        let action = match (detected == Some(info.target()), downgrade) {
            (true, true) => SlotAction::KeepDowngrade,
            (true, false) => SlotAction::Copy,
            (false, true) => SlotAction::ConvertDowngrade,
            (false, false) => SlotAction::Convert,
        };

        decisions.push((relative.clone(), action));
    }

    Ok(decisions)
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

/// Lightweight destination check for `--dry-run`.
///
/// A dry run must also work on a folder that was already converted, so an
/// occupied destination is returned as a warning instead of an error.
pub fn output_warning(input: &Path, output: &Path) -> Option<String> {
    let absolute = absolute_path(output).ok()?;
    let input_canonical = fs::canonicalize(input).ok()?;

    if absolute.starts_with(&input_canonical) {
        return Some(
            "the output folder cannot be inside the input folder; a real run would refuse"
                .to_owned(),
        );
    }

    if absolute.exists() {
        return Some(format!(
            "'{}' already exists; a real run would refuse to overwrite it",
            absolute.display()
        ));
    }

    None
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

/// Checks a `--option-sav` replacement before anything is written.
pub fn validate_option_override(path: &Path, target: Platform) -> Result<Vec<u8>> {
    let header = read_header(path)?.ok_or_else(|| {
        Error::InvalidInput(format!("'{}' is shorter than 4 bytes", path.display()))
    })?;
    let detected = Platform::from_magic(&header).ok_or_else(|| {
        Error::InvalidInput(format!(
            "'{}' is not a Breath of the Wild option.sav",
            path.display()
        ))
    })?;

    if detected != target {
        return Err(Error::InvalidInput(format!(
            "'{}' is a {} option.sav, but the output is {}",
            path.display(),
            detected.label(),
            target.label()
        )));
    }

    if target == Platform::WiiU {
        if let Some(raw) = detected.version_code(&header) {
            if !Platform::is_wii_u_safe(raw) {
                return Err(Error::InvalidInput(format!(
                    "'{}' is {}, which a Wii U copy of the game cannot read",
                    path.display(),
                    Platform::version_label(raw)
                )));
            }
        }
    }

    fs::read(path).map_err(|error| Error::io("read option.sav replacement", path, error))
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
    source: &SaveInfo,
    requested_output: &Path,
    options: &ConvertOptions,
) -> Result<ConvertReport> {
    let blockers = source.blockers(options);
    if !blockers.is_empty() {
        return Err(Error::UnsupportedSave(blockers.join("\n  ")));
    }

    let output = validate_output_path(&source.root, requested_output, true)?;

    let replacement = match &options.option_sav {
        Some(path) => Some(validate_option_override(path, source.target())?),
        None => None,
    };

    let dropped_dirs: Vec<PathBuf> = source
        .slots_to_drop(options)
        .iter()
        .map(|slot| PathBuf::from(&slot.directory))
        .collect();

    let temporary = TemporaryDirectory::create(&output)?;
    let files_copied = copy_directory(&source.root, temporary.path(), &dropped_dirs)?;

    if let Some(data) = &replacement {
        let path = temporary.path().join(OPTION_FILE);
        fs::write(&path, data)
            .map_err(|error| Error::io("write replacement option.sav", &path, error))?;
    }

    let mut files_converted = 0usize;
    let mut files_kept = 0usize;
    let mut files_downgraded = 0usize;
    let mut removed_fields = 0usize;

    for relative_path in &source.sav_files {
        if dropped_dirs
            .iter()
            .any(|dir| relative_path.starts_with(dir))
        {
            continue;
        }

        let path = temporary.path().join(relative_path);
        let mut data =
            fs::read(&path).map_err(|error| Error::io("read save file", &path, error))?;
        let original_len = data.len();
        let is_trackblock = path.to_string_lossy().contains("trackblock");
        let kind = relative_path
            .file_name()
            .and_then(OsStr::to_str)
            .and_then(Kind::from_name);
        let needs_downgrade = source.target() == Platform::WiiU
            && header_version(&data).is_some_and(Platform::needs_downgrade);

        // A file that already carries the target platform's byte order was
        // converted earlier (or imported from the other platform). Swapping it
        // again would corrupt it, so it is only rewritten if it still carries
        // fields the target release does not know.
        if data.len() >= 4 && Platform::from_magic(&data[..4]) == Some(source.target()) {
            if needs_downgrade {
                let kind = downgrade_kind(kind, relative_path)?;
                removed_fields += apply_downgrade(&mut data, kind, relative_path)?;
                files_downgraded += 1;
                fs::write(&path, &data)
                    .map_err(|error| Error::io("write downgraded save", &path, error))?;
            }
            files_kept += 1;
            continue;
        }

        convert_sav_bytes(&mut data, is_trackblock);

        if needs_downgrade {
            let kind = downgrade_kind(kind, relative_path)?;
            removed_fields += apply_downgrade(&mut data, kind, relative_path)?;
            files_downgraded += 1;
        }

        if data.len() > original_len {
            return Err(Error::Verification(format!(
                "the size of '{}' grew during conversion",
                relative_path.display()
            )));
        }

        fs::write(&path, &data).map_err(|error| Error::io("write converted save", &path, error))?;
        files_converted += 1;
    }

    verify_converted_save(temporary.path(), source, source.target(), &dropped_dirs)?;

    let target = source.target();
    temporary.persist(&output)?;

    Ok(ConvertReport {
        target,
        files_copied,
        files_converted,
        files_kept,
        files_downgraded,
        removed_fields,
        dropped: dropped_dirs,
        option_replaced: replacement.is_some(),
        output,
    })
}

fn verify_converted_save(
    output: &Path,
    source: &SaveInfo,
    target: Platform,
    dropped: &[PathBuf],
) -> Result<()> {
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

    let expected_slots = source
        .slots
        .iter()
        .filter(|slot| {
            !dropped
                .iter()
                .any(|dir| slot.relative_path.starts_with(dir))
        })
        .count();

    if converted.slots.len() != expected_slots {
        return Err(Error::Verification(format!(
            "expected {expected_slots} slots in the output, found {}",
            converted.slots.len()
        )));
    }

    for source_slot in &source.slots {
        if dropped
            .iter()
            .any(|dir| source_slot.relative_path.starts_with(dir))
        {
            continue;
        }

        let Some(expected) = source_slot.raw_version else {
            continue;
        };
        let expected = if target == Platform::WiiU && Platform::needs_downgrade(expected) {
            WII_U_SAFE_VERSION
        } else {
            expected
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

        if converted_slot.raw_version != Some(expected) {
            return Err(Error::Verification(format!(
                "game version changed in '{}' (was 0x{expected:04x}, now {})",
                source_slot.relative_path.display(),
                converted_slot
                    .raw_version
                    .map(|raw| format!("0x{raw:04x}"))
                    .unwrap_or_else(|| "unknown".to_owned())
            )));
        }

        if converted_slot.platform != Some(target) {
            return Err(Error::Verification(format!(
                "'{}' still uses {} byte order after conversion",
                source_slot.relative_path.display(),
                converted_slot.platform_label()
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

/// Recognises duplicates created by cloud sync or file managers, e.g.
/// `game_data-NSConflict-5-mac15.6.sav` or `caption-冲突-Jeasin_Win11.sav`.
fn is_conflict_copy(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.contains("NSConflict")
        || name.contains("冲突")
        || lower.contains("-copy")
        || lower.contains(" copy")
        || lower.ends_with(".bak")
}

/// Reads the version code from a file's own header, whatever byte order it is
/// currently stored in.
fn header_version(data: &[u8]) -> Option<u16> {
    let header = data.get(..4)?;
    Platform::from_magic(header)?.version_code(header)
}

/// Refuses to leave a newer header in place on a file this build cannot rewrite.
fn downgrade_kind(kind: Option<Kind>, relative: &Path) -> Result<Kind> {
    kind.ok_or_else(|| {
        Error::UnsupportedSave(format!(
            "'{}' carries a version newer than the Wii U release but is not a record table \
             this build can rewrite",
            relative.display()
        ))
    })
}

/// Drops the fields the Wii U release does not know and retags the header.
/// Returns how many records were removed.
fn apply_downgrade(data: &mut Vec<u8>, kind: Kind, relative: &Path) -> Result<usize> {
    let magic = u32::from(WII_U_SAFE_VERSION);

    match records::downgrade(data, true, kind, magic) {
        Some(rewrite) => {
            *data = rewrite.data;
            Ok(rewrite.removed)
        }
        None => Err(Error::UnsupportedSave(format!(
            "'{}' needs downgrading but its record table could not be read; the container \
             may have changed in that version",
            relative.display()
        ))),
    }
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

fn copy_directory(source: &Path, destination: &Path, skip: &[PathBuf]) -> Result<usize> {
    fn visit(
        source: &Path,
        destination: &Path,
        skip: &[PathBuf],
        prefix: &Path,
        copied: &mut usize,
    ) -> Result<()> {
        let mut entries = fs::read_dir(source)
            .map_err(|error| Error::io("read source directory", source, error))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| Error::io("read source directory entry", source, error))?;
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let relative = prefix.join(entry.file_name());
            let file_type = entry.file_type().map_err(|error| {
                Error::io("inspect source directory entry", &source_path, error)
            })?;

            if skip.iter().any(|dir| relative.starts_with(dir)) {
                continue;
            }

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
                visit(&source_path, &destination_path, skip, &relative, copied)?;
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
    visit(source, destination, skip, Path::new(""), &mut copied)?;
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

        match fs::rename(&self.path, target) {
            Ok(()) => {
                self.armed = false;
                Ok(())
            }
            // macOS denies the rename itself in two situations that still allow
            // writing files: `~/Downloads` and `~/Desktop` sit behind a privacy
            // prompt, and sandboxed shells deny `rename` with EPERM while
            // allowing `create` and `write`. Copying the finished tree keeps the
            // output where the user asked for it; `target` was proven absent
            // above, so nothing can be overwritten.
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                fs::create_dir(target).map_err(|error| {
                    Error::io(
                        "create the output folder - macOS may be blocking this location, \
                         check System Settings > Privacy & Security > Files and Folders",
                        target,
                        error,
                    )
                })?;

                if let Err(error) = copy_directory(&self.path, target, &[]) {
                    let _ = fs::remove_dir_all(target);
                    return Err(error);
                }

                let _ = fs::remove_dir_all(&self.path);
                self.armed = false;
                Ok(())
            }
            Err(error) => Err(Error::io("move converted save into place", target, error)),
        }
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
