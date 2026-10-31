// SPDX-License-Identifier: AGPL-3.0-only

//! The small `caption.sav` next to each `game_data.sav`.
//!
//! It uses the same record container as the other files (header, `{hash,
//! value}` records, `0xffffffff` terminator) but holds only what the
//! save-select screen shows beside the screenshot: when the save was written,
//! how long that file has been played, where Link was standing, and two flags
//! that separate the six normal-mode slots from the two Master Mode ones.
//!
//! Field hashes were identified on real saves by pairing the stored values with
//! what the game displays: the timestamp and play time match the on-screen
//! figures, and the two flags split the eight slots exactly the way the game
//! does (one manual save plus five autosaves in normal mode, one of each in
//! Master Mode).

use crate::platform::Platform;

/// Unix timestamp of the moment the save was written.
const SAVED_AT: u32 = 0x0ba7_cf40;
/// Play time of this save file, in minutes.
const PLAY_MINUTES: u32 = 0xa067_8286;
/// 1 on the two Master Mode slots, 0 on the six normal-mode slots.
const MASTER_MODE: u32 = 0xe512_56e2;
/// 0 on the single manual save of each mode, 1 on the autosave slots.
const AUTOSAVE: u32 = 0x4acb_5004;
/// Place name, stored as a run of four-byte chunks.
const PLACE: u32 = 0x46d2_66b6;
/// `MapArea_*` region id, same chunk encoding.
const AREA: u32 = 0xf26f_1a02;

/// What the save-select screen knows about one slot.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Caption {
    /// Unix seconds, as the game's own clock reported them.
    pub saved_at: Option<u32>,
    /// Play time of this file, in minutes.
    pub play_minutes: Option<u32>,
    /// `Some(true)` for the two Master Mode slots.
    pub master_mode: Option<bool>,
    /// `Some(true)` when this is the mode's manual save rather than an autosave.
    pub manual: Option<bool>,
    /// Display name of the spot, when the game stored one.
    pub place: Option<String>,
    /// `MapArea_*` id of the region, when the game stored one.
    pub area: Option<String>,
}

impl Caption {
    /// Reads a `caption.sav`. Returns `None` when the file is too short or is
    /// not a record table, so a missing or damaged caption never blocks the
    /// conversion itself.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let platform = Platform::from_magic(data.get(..4)?)?;
        if data.len() < 16 {
            return None;
        }

        let big_endian = platform == Platform::WiiU;
        let count = (data.len() - 16) / 8;
        let mut caption = Self::default();
        let mut place = Vec::new();
        let mut area = Vec::new();

        for index in 0..count {
            let offset = 12 + index * 8;
            let hash = read_u32(data, offset, big_endian);
            let value = read_u32(data, offset + 4, big_endian);

            match hash {
                SAVED_AT => caption.saved_at = Some(value),
                PLAY_MINUTES => caption.play_minutes = Some(value),
                MASTER_MODE => caption.master_mode = Some(value != 0),
                AUTOSAVE => caption.manual = Some(value == 0),
                // String chunks keep their byte order on both platforms, so the
                // raw four bytes are appended instead of the decoded word.
                PLACE => place.extend_from_slice(&data[offset + 4..offset + 8]),
                AREA => area.extend_from_slice(&data[offset + 4..offset + 8]),
                _ => {}
            }
        }

        caption.place = clean_text(&place);
        caption.area = clean_text(&area);
        Some(caption)
    }

    /// `2026-10-22 22:23`, or `None` when the stored value is not a plausible
    /// Unix timestamp. Rendered as UTC: emulators seed the console clock with
    /// the host's wall clock treated as UTC, so this is the time the player saw.
    pub fn saved_label(&self) -> Option<String> {
        let seconds = i64::from(self.saved_at?);
        if !(946_684_800..4_102_444_800).contains(&seconds) {
            return None;
        }

        let days = seconds.div_euclid(86_400);
        let rest = seconds.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        Some(format!(
            "{year:04}-{month:02}-{day:02} {:02}:{:02}",
            rest / 3600,
            (rest % 3600) / 60
        ))
    }

    /// `141:05` — hours and minutes, the way the load screen shows play time.
    pub fn play_label(&self) -> Option<String> {
        let minutes = self.play_minutes?;
        Some(format!("{}:{:02}", minutes / 60, minutes % 60))
    }

    /// `normal auto`, `master manual`, ... or `-` when the flags are missing.
    pub fn kind_label(&self) -> &'static str {
        match (self.master_mode, self.manual) {
            (Some(true), Some(true)) => "master manual",
            (Some(true), Some(false)) => "master auto",
            (Some(false), Some(true)) => "normal manual",
            (Some(false), Some(false)) => "normal auto",
            (Some(true), None) => "master",
            (Some(false), None) => "normal",
            _ => "-",
        }
    }

    /// Region name for the slot table: `MapArea_CentralHyrule` is shown as
    /// `Central Hyrule`, falling back to the place name when the region id is
    /// missing.
    pub fn location_label(&self) -> Option<String> {
        if let Some(area) = &self.area {
            if let Some(name) = area.strip_prefix("MapArea_") {
                return Some(split_camel_case(name));
            }
            return Some(area.clone());
        }

        self.place
            .as_deref()
            .filter(|place| !place.chars().any(|ch| ch.is_ascii_digit()))
            .map(split_camel_case)
    }
}

fn read_u32(data: &[u8], offset: usize, big_endian: bool) -> u32 {
    let bytes = [
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ];
    if big_endian {
        u32::from_be_bytes(bytes)
    } else {
        u32::from_le_bytes(bytes)
    }
}

/// A string chunk run is NUL-padded to a fixed width; keep what precedes the
/// first NUL and reject anything that is not plain text.
fn clean_text(bytes: &[u8]) -> Option<String> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let text = std::str::from_utf8(&bytes[..end]).ok()?.trim();
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }

    Some(text.to_owned())
}

/// `CentralHyrule` -> `Central Hyrule`.
fn split_camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if index > 0 && ch.is_ascii_uppercase() {
            if let Some(previous) = out.chars().last() {
                if previous.is_ascii_lowercase() {
                    out.push(' ');
                }
            }
        }
        out.push(ch);
    }

    out
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm), so the
/// tool stays free of date-formatting dependencies.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;

    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::{Caption, civil_from_days, split_camel_case};

    const SAVED_AT: u32 = 0x0ba7_cf40;
    const PLAY_MINUTES: u32 = 0xa067_8286;
    const MASTER_MODE: u32 = 0xe512_56e2;
    const AUTOSAVE: u32 = 0x4acb_5004;
    const PLACE: u32 = 0x46d2_66b6;
    const AREA: u32 = 0xf26f_1a02;

    /// One field of a synthetic caption file.
    enum Field<'a> {
        Word(u32),
        /// A string chunk run, NUL-padded to the given number of four-byte
        /// chunks. String bytes keep their order on both platforms.
        Text(&'a str, usize),
    }

    /// Builds a `caption.sav` the way the game lays it out.
    fn caption_file(big_endian: bool, records: &[(u32, Field<'_>)]) -> Vec<u8> {
        let push = |data: &mut Vec<u8>, value: u32| {
            if big_endian {
                data.extend_from_slice(&value.to_be_bytes());
            } else {
                data.extend_from_slice(&value.to_le_bytes());
            }
        };

        let mut data = Vec::new();
        push(&mut data, 0x471b);
        push(&mut data, 0xffff_ffff);
        push(&mut data, 1);
        for (hash, field) in records {
            match field {
                Field::Word(value) => {
                    push(&mut data, *hash);
                    push(&mut data, *value);
                }
                // A string is one record per four bytes, the hash repeating.
                Field::Text(text, chunks) => {
                    let mut bytes = text.as_bytes().to_vec();
                    bytes.resize(chunks * 4, 0);
                    for chunk in bytes.chunks(4) {
                        push(&mut data, *hash);
                        data.extend_from_slice(chunk);
                    }
                }
            }
        }
        push(&mut data, 0xffff_ffff);
        data
    }

    #[test]
    fn reads_the_flags_the_save_select_screen_shows() {
        let records = [
            (SAVED_AT, Field::Word(1_792_679_016)),
            (PLAY_MINUTES, Field::Word(8661)),
            (MASTER_MODE, Field::Word(1)),
            (AUTOSAVE, Field::Word(0)),
            (AREA, Field::Text("MapArea_HyruleForest", 24)),
        ];

        for big_endian in [false, true] {
            let data = caption_file(big_endian, &records);
            let caption = Caption::parse(&data).expect("caption parses");
            assert_eq!(caption.saved_label().as_deref(), Some("2026-10-22 14:23"));
            assert_eq!(caption.play_label().as_deref(), Some("144:21"));
            assert_eq!(caption.kind_label(), "master manual");
            assert_eq!(caption.location_label().as_deref(), Some("Hyrule Forest"));
        }
    }

    #[test]
    fn distinguishes_autosaves_from_the_manual_save() {
        let auto = caption_file(
            false,
            &[(MASTER_MODE, Field::Word(0)), (AUTOSAVE, Field::Word(1))],
        );
        let manual = caption_file(
            false,
            &[(MASTER_MODE, Field::Word(0)), (AUTOSAVE, Field::Word(0))],
        );

        assert_eq!(Caption::parse(&auto).unwrap().kind_label(), "normal auto");
        assert_eq!(
            Caption::parse(&manual).unwrap().kind_label(),
            "normal manual"
        );
    }

    #[test]
    fn falls_back_to_the_place_name_when_the_region_is_missing() {
        let data = caption_file(true, &[(PLACE, Field::Text("Hateno", 16))]);
        let caption = Caption::parse(&data).expect("caption parses");

        assert_eq!(caption.location_label().as_deref(), Some("Hateno"));
    }

    #[test]
    fn missing_fields_stay_empty_instead_of_guessing() {
        let bare = caption_file(true, &[]);
        let caption = Caption::parse(&bare).expect("header is enough");

        assert_eq!(caption.saved_label(), None);
        assert_eq!(caption.play_label(), None);
        assert_eq!(caption.kind_label(), "-");
        assert_eq!(caption.location_label(), None);
    }

    #[test]
    fn rejects_files_that_are_not_caption_tables() {
        assert!(Caption::parse(&[]).is_none());
        assert!(Caption::parse(&[0, 0, 0, 0, 0, 0, 0, 0]).is_none());
        assert!(Caption::parse(&[0x1b, 0x47, 0x00, 0x00, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn formats_dates_without_a_date_library() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_661), (2026, 7, 27));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn splits_camel_case_region_ids() {
        assert_eq!(split_camel_case("CentralHyrule"), "Central Hyrule");
        assert_eq!(split_camel_case("EldinCanyon"), "Eldin Canyon");
        assert_eq!(split_camel_case("WestHateru"), "West Hateru");
    }
}
