// SPDX-License-Identifier: AGPL-3.0-only

const HASHES: [u32; 44] = [
    0x7b74_e117,
    0x17e1_747b,
    0xd913_b769,
    0x69b7_13d9,
    0xb666_d246,
    0x46d2_66b6,
    0x021a_6ff2,
    0xf26f_1a02,
    0xff74_960f,
    0x0f96_74ff,
    0x8932_285f,
    0x5f28_3289,
    0x3b0a_289b,
    0x9b28_0a3b,
    0x2f95_768f,
    0x8f76_952f,
    0x9c6c_fd3f,
    0x3ffd_6c9c,
    0xbbac_416b,
    0x6b41_acbb,
    0xccab_71fd,
    0xfd71_abcc,
    0xcbc6_b5e4,
    0xe4b5_c6cb,
    0x2cad_b0e7,
    0xe7b0_ad2c,
    0xa6eb_3ef4,
    0xf43e_eba6,
    0x21d4_cffa,
    0xfacf_d421,
    0x22a5_10d1,
    0xd110_a522,
    0x98d1_0d53,
    0x530d_d198,
    0x55a2_2047,
    0x4720_a255,
    0xe5a6_3a33,
    0x333a_a6e5,
    0xbec6_5061,
    0x6150_c6be,
    0xbc11_8370,
    0x7083_11bc,
    0x0e9d_0e75,
    0x750e_9d0e,
];

const ITEMS: [&[u8]; 60] = [
    b"Item", b"Weap", b"Armo", b"Fire", b"Norm", b"IceA", b"Elec", b"Bomb", b"Anci", b"Anim",
    b"Obj_", b"Game", b"Dm_N", b"Dm_A", b"Dm_E", b"Dm_P", b"FldO", b"Gano", b"Gian", b"Grea",
    b"KeyS", b"Kokk", b"Liza", b"Mann", b"Mori", b"Npc_", b"OctO", b"Octa", b"Octa", b"arro",
    b"Pict", b"PutR", b"Rema", b"Site", b"TBox", b"TwnO", b"Prie", b"Dye0", b"Dye1", b"Map",
    b"Play", b"Oasi", b"Cele", b"Wolf", b"Gata", b"Ston", b"Kaka", b"Soji", b"Hyru", b"Powe",
    b"Lana", b"Hate", b"Akka", b"Yash", b"Dung", b"BeeH", b"Boar", b"Boko", b"Brig", b"DgnO",
];

/// Converts one .sav file in place by swapping the endianness of the words
/// that contain numeric data. This is a direct port of the original
/// `BotwSave.ConvertPlatform` loop.
pub fn convert_sav_bytes(data: &mut [u8], is_trackblock: bool) {
    let word_count = data.len() / 4;
    let mut pos = 0usize;

    while pos < word_count {
        // trackblock files have a special two-byte field at offset 4.
        if is_trackblock && pos == 0 {
            if data.len() >= 6 {
                data[4..6].reverse();
            }
            pos = 2;
            if pos >= word_count {
                break;
            }
        }

        let offset = pos * 4;
        let buffer = [
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ];
        let value = u32::from_le_bytes(buffer);

        if HASHES.contains(&value) {
            data[offset..offset + 4].reverse();
            pos += 2;
        } else if ITEMS
            .iter()
            .any(|item| buffer.windows(item.len()).any(|window| window == *item))
        {
            pos += 1;

            // The original algorithm converts 16 alternating words after an
            // item header, then skips 30 words.
            for index in 0..16 {
                let current = pos + index * 2;
                if current >= word_count {
                    break;
                }
                let current_offset = current * 4;
                data[current_offset..current_offset + 4].reverse();
            }

            pos += 31;
        } else {
            data[offset..offset + 4].reverse();
            pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HASHES, convert_sav_bytes};

    fn words(values: &[[u8; 4]]) -> Vec<u8> {
        values.iter().flatten().copied().collect()
    }

    #[test]
    fn reverses_normal_words_and_leaves_remainder() {
        let mut data = vec![1, 2, 3, 4, 5];
        convert_sav_bytes(&mut data, false);
        assert_eq!(data, vec![4, 3, 2, 1, 5]);
    }

    #[test]
    fn reverses_known_hash_and_skips_the_following_word() {
        let mut data = words(&[HASHES[0].to_le_bytes(), [1, 2, 3, 4]]);
        convert_sav_bytes(&mut data, false);

        let mut expected = HASHES[0].to_le_bytes();
        expected.reverse();
        assert_eq!(&data[..4], &expected);
        assert_eq!(&data[4..], &[1, 2, 3, 4]);
    }

    #[test]
    fn converts_item_regions_without_swapping_the_header() {
        let mut input = vec![[0u8; 4]; 33];
        input[0] = *b"Item";
        for (index, word) in input.iter_mut().enumerate().skip(1) {
            *word = (index as u32).to_le_bytes();
        }

        let mut data = words(&input);
        convert_sav_bytes(&mut data, false);

        assert_eq!(&data[..4], b"Item");
        for (index, word) in input.iter().enumerate().skip(1) {
            let offset = index * 4;
            let mut expected = *word;
            if index % 2 == 1 || index == 32 {
                expected.reverse();
            }
            assert_eq!(&data[offset..offset + 4], &expected, "word {index}");
        }
    }

    #[test]
    fn handles_trackblock_header_before_the_main_loop() {
        let mut data = vec![
            0x00, 0x00, 0x00, 0x00, // word 0: skipped
            0xaa, 0xbb, 0xcc, 0xdd, // two bytes at offset 4 are swapped
            0x01, 0x02, 0x03, 0x04, // word 2: normal
            0x05, 0x06, 0x07, 0x08, // word 3: normal
        ];
        convert_sav_bytes(&mut data, true);

        assert_eq!(
            data,
            vec![
                0x00, 0x00, 0x00, 0x00, 0xbb, 0xaa, 0xcc, 0xdd, 0x04, 0x03, 0x02, 0x01, 0x08, 0x07,
                0x06, 0x05,
            ]
        );
    }
}
