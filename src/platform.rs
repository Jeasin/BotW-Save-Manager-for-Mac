// SPDX-License-Identifier: AGPL-3.0-only

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    WiiU,
    Switch,
}

const VERSION_HEADERS: [(u16, &str); 8] = [
    (0x24e2, "v1.0"),
    (0x24ee, "v1.1"),
    (0x2588, "v1.2"),
    (0x29c0, "v1.3"),
    (0x3ef8, "v1.3.3"),
    (0x471a, "v1.4"),
    (0x471b, "v1.5"),
    (0x471e, "v1.6"),
];

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Self::WiiU => "Wii U",
            Self::Switch => "Switch",
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

    pub fn from_magic(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }

        match u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) {
            0x1b47_0000 => Some(Self::WiiU),
            0x0000_471b => Some(Self::Switch),
            _ => None,
        }
    }

    /// Reads the game version from the first four bytes of game_data.sav.
    ///
    /// This mirrors the original C# implementation: Wii U headers are read as
    /// a big-endian 32-bit value and Switch headers as a little-endian value.
    pub fn version_from_header(self, header: &[u8]) -> Option<&'static str> {
        if header.len() < 4 {
            return None;
        }

        let raw = match self {
            Self::Switch => u16::from_le_bytes([header[0], header[1]]),
            Self::WiiU => u16::from_le_bytes([header[3], header[2]]),
        };

        VERSION_HEADERS
            .iter()
            .find_map(|(value, name)| (*value == raw).then_some(*name))
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
        assert_eq!(Platform::from_magic(&[0x00, 0x00, 0x00, 0x00]), None);
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
    }

    #[test]
    fn round_trips_platform_magic() {
        for platform in [Platform::WiiU, Platform::Switch] {
            assert_eq!(Platform::from_magic(&platform.magic()), Some(platform));
        }
    }
}
