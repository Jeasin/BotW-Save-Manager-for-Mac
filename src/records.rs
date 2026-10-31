// SPDX-License-Identifier: AGPL-3.0-only

//! Record-table container used by `option.sav`, `game_data.sav` and `caption.sav`.
//!
//! All three files share one layout:
//!
//! ```text
//! 0x00  u32   version magic (0x471b on Wii U 1.5.0, 0x4730 on newer Switch)
//! 0x04  u32   unchanged
//! 0x08  u32   unchanged
//! 0x0c  ...   { u32 hash; u32 value } repeated
//! end-4 u32   0xffffffff terminator
//! ```
//!
//! A hash can repeat: arrays and strings are stored as a run of records that
//! share one hash, each record carrying four more bytes. Numeric words change
//! byte order between platforms, the four bytes of a string chunk do not.
//!
//! Because the file is keyed by hash rather than by fixed offsets, a newer save
//! can be turned back into an older one by dropping the fields the older
//! release does not know and retagging the header.

pub const HEADER_LEN: usize = 0x0c;
pub const RECORD_LEN: usize = 8;
pub const TRAILER_LEN: usize = 4;
pub const TERMINATOR: u32 = 0xffff_ffff;

/// Fields that exist in `game_data.sav` only after Wii U 1.5.0.
///
/// `0xf3316abb` arrived with v1.6 (Switch-only, Labo VR). The other four are
/// new in the Switch 2 Edition format. Removing exactly these five turns a
/// newer `game_data.sav` back into the record set of a native v1.5.0 save:
/// measured on real saves, every known hash lands on its v1.5 offset again.
pub const NEWER_GAME_DATA_FIELDS: [u32; 5] = [
    0x6213_3c14,
    0x8daf_a42c,
    0xcf03_1efd,
    0xf331_6abb,
    0xfa05_0b46,
];

/// Fields that exist in `option.sav` only after Wii U 1.5.0.
///
/// Dropping them restores the 40-record, 336-byte layout the Wii U release
/// writes.
pub const NEWER_OPTION_FIELDS: [u32; 10] = [
    0x1c8a_3035,
    0x2222_1c03,
    0x226d_d1ef,
    0x448c_ee32,
    0x4e5f_f3e2,
    0x6411_584e,
    0x679d_7f27,
    0x85fd_1acc,
    0x9326_cb93,
    0xb49b_ce05,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    GameData,
    Option,
    Caption,
}

impl Kind {
    /// Classifies a file name. `None` means the file is not a record table
    /// (for example a `trackblock*.sav` tracker file).
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();

        if lower == "game_data.sav" {
            Some(Self::GameData)
        } else if lower == "option.sav" {
            Some(Self::Option)
        } else if lower.starts_with("caption") && lower.ends_with(".sav") {
            Some(Self::Caption)
        } else {
            None
        }
    }

    /// Fields this file type gained after Wii U 1.5.0.
    pub fn newer_fields(self) -> &'static [u32] {
        match self {
            Self::GameData => &NEWER_GAME_DATA_FIELDS,
            Self::Option => &NEWER_OPTION_FIELDS,
            Self::Caption => &[],
        }
    }
}

pub struct Table<'a> {
    pub magic: u32,
    /// Bytes 4..12, carried through unchanged.
    pub preamble: &'a [u8],
    pub records: Vec<(u32, u32)>,
    pub trailer: &'a [u8],
}

impl<'a> Table<'a> {
    /// Parses the container, or returns `None` when the file does not have the
    /// expected shape (wrong size, missing terminator, too short).
    pub fn parse(data: &'a [u8], big_endian: bool) -> Option<Self> {
        if data.len() < HEADER_LEN + RECORD_LEN + TRAILER_LEN {
            return None;
        }

        let body_len = data.len() - HEADER_LEN - TRAILER_LEN;
        if body_len % RECORD_LEN != 0 {
            return None;
        }

        let trailer_start = data.len() - TRAILER_LEN;
        if read_u32(data, trailer_start, big_endian) != TERMINATOR {
            return None;
        }

        let mut records = Vec::with_capacity(body_len / RECORD_LEN);
        let mut offset = HEADER_LEN;
        while offset < trailer_start {
            records.push((
                read_u32(data, offset, big_endian),
                read_u32(data, offset + 4, big_endian),
            ));
            offset += RECORD_LEN;
        }

        Some(Self {
            magic: read_u32(data, 0, big_endian),
            preamble: &data[4..HEADER_LEN],
            records,
            trailer: &data[trailer_start..],
        })
    }

    pub fn to_bytes(&self, big_endian: bool) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(HEADER_LEN + self.records.len() * RECORD_LEN + self.trailer.len());

        push_u32(&mut out, self.magic, big_endian);
        out.extend_from_slice(self.preamble);
        for (hash, value) in &self.records {
            push_u32(&mut out, *hash, big_endian);
            push_u32(&mut out, *value, big_endian);
        }
        out.extend_from_slice(self.trailer);

        out
    }
}

pub struct Downgrade {
    pub data: Vec<u8>,
    /// How many records were removed.
    pub removed: usize,
    pub previous_magic: u32,
}

/// Rewrites a record table so the Wii U 1.5.0 release can read it: drops the
/// fields that did not exist back then and retags the file with `magic`.
///
/// Returns `None` when the file is not a record table at all.
pub fn downgrade(data: &[u8], big_endian: bool, kind: Kind, magic: u32) -> Option<Downgrade> {
    let table = Table::parse(data, big_endian)?;
    let drop = kind.newer_fields();
    let total = table.records.len();
    let mut kept = Vec::with_capacity(total);

    for record in table.records {
        if !drop.contains(&record.0) {
            kept.push(record);
        }
    }

    let mut out = Vec::with_capacity(HEADER_LEN + kept.len() * RECORD_LEN + TRAILER_LEN);
    push_u32(&mut out, magic, big_endian);
    out.extend_from_slice(table.preamble);
    for (hash, value) in &kept {
        push_u32(&mut out, *hash, big_endian);
        push_u32(&mut out, *value, big_endian);
    }
    out.extend_from_slice(table.trailer);

    Some(Downgrade {
        data: out,
        removed: total - kept.len(),
        previous_magic: table.magic,
    })
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

fn push_u32(out: &mut Vec<u8>, value: u32, big_endian: bool) {
    if big_endian {
        out.extend_from_slice(&value.to_be_bytes());
    } else {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, NEWER_GAME_DATA_FIELDS, Table, downgrade, push_u32};

    fn build(magic: u32, records: &[(u32, u32)], big_endian: bool) -> Vec<u8> {
        let mut out = Vec::new();
        push_u32(&mut out, magic, big_endian);
        push_u32(&mut out, 0xffff_ffff, big_endian);
        push_u32(&mut out, 1, big_endian);
        for (hash, value) in records {
            push_u32(&mut out, *hash, big_endian);
            push_u32(&mut out, *value, big_endian);
        }
        push_u32(&mut out, 0xffff_ffff, big_endian);
        out
    }

    #[test]
    fn parses_and_rebuilds_identical_bytes() {
        for big_endian in [false, true] {
            let data = build(0x471b, &[(1, 2), (3, 4), (3, 5)], big_endian);
            let table = Table::parse(&data, big_endian).expect("table");

            assert_eq!(table.magic, 0x471b);
            assert_eq!(table.records, vec![(1, 2), (3, 4), (3, 5)]);
            assert_eq!(table.to_bytes(big_endian), data);
        }
    }

    #[test]
    fn rejects_files_that_are_not_record_tables() {
        assert!(Table::parse(&[0u8; 3], false).is_none());
        assert!(Table::parse(&[0u8; 64], false).is_none());

        let mut bad = build(0x471b, &[(1, 2)], false);
        let last = bad.len() - 1;
        bad[last] = 0;
        assert!(Table::parse(&bad, false).is_none());
    }

    #[test]
    fn downgrade_drops_newer_fields_and_retags() {
        let newer = NEWER_GAME_DATA_FIELDS[0];
        let data = build(0x4730, &[(0x10, 1), (newer, 7), (0x20, 2)], false);

        let downgraded = downgrade(&data, false, Kind::GameData, 0x471b).expect("downgrade");

        assert_eq!(downgraded.removed, 1);
        assert_eq!(downgraded.previous_magic, 0x4730);
        assert_eq!(
            downgraded.data,
            build(0x471b, &[(0x10, 1), (0x20, 2)], false)
        );
    }

    #[test]
    fn downgrade_keeps_unknown_fields_of_the_same_kind() {
        let data = build(0x4730, &[(0xdead_beef, 1)], true);
        let downgraded = downgrade(&data, true, Kind::Caption, 0x471b).expect("downgrade");

        assert_eq!(downgraded.removed, 0);
        assert_eq!(downgraded.data, build(0x471b, &[(0xdead_beef, 1)], true));
    }

    #[test]
    fn classifies_record_file_names() {
        assert_eq!(Kind::from_name("game_data.sav"), Some(Kind::GameData));
        assert_eq!(Kind::from_name("option.sav"), Some(Kind::Option));
        assert_eq!(Kind::from_name("caption.sav"), Some(Kind::Caption));
        assert_eq!(
            Kind::from_name("caption-NSConflict-5.sav"),
            Some(Kind::Caption)
        );
        assert_eq!(Kind::from_name("trackblock00.sav"), None);
        assert_eq!(Kind::from_name("album.jpg"), None);
    }
}
