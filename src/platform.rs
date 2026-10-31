// SPDX-License-Identifier: AGPL-3.0-only

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    WiiU,
    Switch,
}

/// Last save header the Wii U release of the game ever wrote (v1.5, 0x471b).
///
/// The Wii U version stopped receiving updates at 1.5.0; Nintendo states that
/// nothing from 1.6.0 onwards was released for it. Every later header therefore
/// describes a Switch-only layout: 0x471e is v1.6.0 (Switch, Labo VR) and
/// 0x4730 covers v1.8.x / v1.9.x, the updates that shipped alongside the Switch
/// 2 (Thai text, bug fixes) but which the plain Switch also receives.
pub const WII_U_SAFE_VERSION: u16 = 0x471b;

const VERSION_HEADERS: [(u16, &str); 9] = [
    (0x24e2, "v1.0"),
    (0x24ee, "v1.1"),
    (0x2588, "v1.2"),
    (0x29c0, "v1.3"),
    (0x3ef8, "v1.3.3"),
    (0x471a, "v1.4"),
    (0x471b, "v1.5"),
    (0x471e, "v1.6"),
    (0x4730, "v1.8/v1.9"),
];

/// Headers whose extra fields this build knows by hash.
const KNOWN_NEWER_HEADERS: [u16; 2] = [0x471e, 0x4730];

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Self::WiiU => "Wii U",
            Self::Switch => "Switch",
        }
    }

    /// Label used in user-facing status lines.
    pub fn display_label(self) -> &'static str {
        match self {
            Self::WiiU => "Wii U",
            Self::Switch => "Switch (NS)",
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::WiiU => Self::Switch,
            Self::Switch => Self::WiiU,
        }
    }

    pub fn magic(self) -> [u8; 4] {
        match self {
            Self::WiiU => [0x00, 0x00, 0x47, 0x1b],
            Self::Switch => [0x1b, 0x47, 0x00, 0x00],
        }
    }

    /// Identifies the platform from an `option.sav` / `game_data.sav` header.
    ///
    /// Both platforms store the same 16-bit version code, only the byte order
    /// differs: Wii U writes `00 00 47 xx`, Switch writes `xx 47 00 00`.
    /// Matching the shape instead of a fixed list keeps newer versions such as
    /// the v1.8/v1.9 layout (`xx = 0x30`) detectable.
    pub fn from_magic(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }

        if bytes[0] == 0 && bytes[1] == 0 && bytes[2] == 0x47 {
            Some(Self::WiiU)
        } else if bytes[1] == 0x47 && bytes[2] == 0 && bytes[3] == 0 {
            Some(Self::Switch)
        } else {
            None
        }
    }

    /// Reads the raw 16-bit version code from the first four bytes of a save.
    pub fn version_code(self, header: &[u8]) -> Option<u16> {
        if header.len() < 4 {
            return None;
        }

        Some(match self {
            Self::Switch => u16::from_le_bytes([header[0], header[1]]),
            Self::WiiU => u16::from_le_bytes([header[3], header[2]]),
        })
    }

    /// Reads the game version from the first four bytes of `game_data.sav`
    /// or `option.sav`.
    pub fn version_from_header(self, header: &[u8]) -> Option<&'static str> {
        let raw = self.version_code(header)?;

        Self::version_name(raw)
    }

    pub fn version_name(raw: u16) -> Option<&'static str> {
        VERSION_HEADERS
            .iter()
            .find_map(|(value, name)| (*value == raw).then_some(*name))
    }

    /// Short label for reports, e.g. `v1.5` or `unknown 0x4731`.
    pub fn version_label(raw: u16) -> String {
        match Self::version_name(raw) {
            Some(name) => name.to_owned(),
            None => format!("unknown 0x{raw:04x}"),
        }
    }

    /// True for versions that a Wii U copy of the game can be expected to read.
    pub fn is_wii_u_safe(raw: u16) -> bool {
        raw <= WII_U_SAFE_VERSION
    }

    /// True for known newer layouts whose extra records can be identified and
    /// removed, which turns the save back into a v1.5 record set.
    ///
    /// 0x471e adds one field, 0x4730 adds five; both keep the same container.
    pub fn needs_downgrade(raw: u16) -> bool {
        raw > WII_U_SAFE_VERSION
    }

    /// True when this build knows exactly which fields the version added, so a
    /// downgrade leaves nothing unknown behind.
    pub fn has_known_layout(raw: u16) -> bool {
        KNOWN_NEWER_HEADERS.contains(&raw)
    }
}

#[cfg(test)]
mod tests {
    use super::Platform;

    #[test]
    fn detects_platform_magic() {
        assert_eq!(
            Platform::from_magic(&[0x00, 0x00, 0x47, 0x1b]),
            Some(Platform::WiiU)
        );
        assert_eq!(
            Platform::from_magic(&[0x1b, 0x47, 0x00, 0x00]),
            Some(Platform::Switch)
        );
        assert_eq!(
            Platform::from_magic(&[0x30, 0x47, 0x00, 0x00]),
            Some(Platform::Switch)
        );
        assert_eq!(Platform::from_magic(&[0x00, 0x00, 0x00, 0x00]), None);
        assert_eq!(Platform::from_magic(&[0x47, 0x1b, 0x00, 0x00]), None);
    }

    #[test]
    fn parses_versions_for_both_platforms() {
        assert_eq!(
            Platform::Switch.version_from_header(&[0x1e, 0x47, 0x00, 0x00]),
            Some("v1.6")
        );
        assert_eq!(
            Platform::WiiU.version_from_header(&[0x00, 0x00, 0x47, 0x1e]),
            Some("v1.6")
        );
        assert_eq!(
            Platform::Switch.version_from_header(&[0x30, 0x47, 0x00, 0x00]),
            Some("v1.8/v1.9")
        );
    }

    #[test]
    fn classifies_versions_by_wii_u_support() {
        assert!(Platform::is_wii_u_safe(0x471b));
        assert!(!Platform::is_wii_u_safe(0x471e));
        assert!(Platform::needs_downgrade(0x471e));
        assert!(Platform::needs_downgrade(0x4730));
        assert!(Platform::needs_downgrade(0x4800), "any newer header");
        assert!(!Platform::needs_downgrade(0x471b));
        assert!(!Platform::needs_downgrade(0x471a));
        assert!(Platform::has_known_layout(0x471e));
        assert!(Platform::has_known_layout(0x4730));
        assert!(!Platform::has_known_layout(0x4800));
    }

    #[test]
    fn labels_unknown_versions_with_the_raw_code() {
        assert_eq!(Platform::version_label(0x471b), "v1.5");
        assert_eq!(Platform::version_label(0x9999), "unknown 0x9999");
    }

    #[test]
    fn round_trips_platform_magic() {
        for platform in [Platform::WiiU, Platform::Switch] {
            assert_eq!(Platform::from_magic(&platform.magic()), Some(platform));
        }
    }
}
