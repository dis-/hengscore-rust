//! Valid-input compatibility with PHP libmbfl SJIS / EUC-JP.
//! JIS tables are derived from PHP; see php-reference/NOTICE and LICENSE.
//! Invalid sequences are rejected rather than substituted as legacy PHP did.

#[derive(Clone, Copy, Debug)]
pub enum Charset {
    Ascii,
    Utf8,
    Sjis,
    EucJp,
}

const JIS0208: &[u8] = include_bytes!("jis0208.bin");
const JIS0212: &[u8] = include_bytes!("jis0212.bin");
const BAD: &str = "malformed encoded text";

fn lookup(table: &[u8], row: usize, column: usize) -> Result<char, &'static str> {
    let index = (row * 94 + column) * 2;
    let pair = table.get(index..index + 2).ok_or(BAD)?;
    let value = u16::from_le_bytes([pair[0], pair[1]]);
    if value == 0 {
        return Err(BAD);
    }
    char::from_u32(u32::from(value)).ok_or(BAD)
}

pub fn decode(charset: Charset, raw: &[u8]) -> Result<String, &'static str> {
    if matches!(charset, Charset::Ascii) {
        if !raw.is_ascii() {
            return Err(BAD);
        }
        return std::str::from_utf8(raw).map(str::to_owned).map_err(|_| BAD);
    }
    if matches!(charset, Charset::Utf8) {
        return std::str::from_utf8(raw).map(str::to_owned).map_err(|_| BAD);
    }
    let mut output = String::with_capacity(raw.len());
    let mut index = 0;
    while let Some(&first) = raw.get(index) {
        index += 1;
        let next = if first < 0x80 {
            char::from(first)
        } else {
            match charset {
                Charset::Sjis if (0xa1..=0xdf).contains(&first) => {
                    char::from_u32(0xfec0 + u32::from(first)).ok_or(BAD)?
                }
                Charset::Sjis => {
                    if !((0x81..=0x9f).contains(&first) || (0xe0..=0xef).contains(&first)) {
                        return Err(BAD);
                    }
                    let second = *raw.get(index).ok_or(BAD)?;
                    index += 1;
                    if !(0x40..=0xfc).contains(&second) || second == 0x7f {
                        return Err(BAD);
                    }
                    let lead = usize::from(first - if first < 0xa0 { 0x81 } else { 0xc1 });
                    let trail = usize::from(second - if second < 0x7f { 0x40 } else { 0x41 });
                    lookup(JIS0208, lead * 2 + trail / 94, trail % 94)?
                }
                Charset::EucJp => {
                    let second = *raw.get(index).ok_or(BAD)?;
                    index += 1;
                    match first {
                        0x8e if (0xa1..=0xdf).contains(&second) => {
                            char::from_u32(0xfec0 + u32::from(second)).ok_or(BAD)?
                        }
                        0x8f => {
                            let third = *raw.get(index).ok_or(BAD)?;
                            index += 1;
                            if !(0xa1..=0xfe).contains(&second) || !(0xa1..=0xfe).contains(&third) {
                                return Err(BAD);
                            }
                            lookup(
                                JIS0212,
                                usize::from(second - 0xa1),
                                usize::from(third - 0xa1),
                            )?
                        }
                        0xa1..=0xfe if (0xa1..=0xfe).contains(&second) => lookup(
                            JIS0208,
                            usize::from(first - 0xa1),
                            usize::from(second - 0xa1),
                        )?,
                        _ => return Err(BAD),
                    }
                }
                Charset::Utf8 | Charset::Ascii => return Err(BAD),
            }
        };
        output.push(next);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_strict_and_preserves_seven_bit_bytes() {
        let bytes: Vec<u8> = (0..=127).collect();
        assert_eq!(decode(Charset::Ascii, &bytes).unwrap().as_bytes(), bytes);
        for byte in 128..=255 {
            assert!(decode(Charset::Ascii, &[b'a', byte, b'z']).is_err());
        }
    }

    #[test]
    fn php_c_decoder_exhaustive_vectors() {
        // Produced by compiling the original PHP C decoder functions, not by
        // the Rust decoder or Python codecs. Includes all 1/2-byte strings and
        // all 0x8f-prefixed 3-byte EUC-JP strings, including invalid sequences.
        let vectors = include_bytes!("../php-reference/compat_vectors.bin");
        let mut cursor = 0;
        let mut cases = 0;
        while cursor < vectors.len() {
            let charset = match vectors[cursor] {
                1 => Charset::Sjis,
                2 => Charset::EucJp,
                _ => panic!("bad fixture charset"),
            };
            let size = usize::from(vectors[cursor + 1]);
            cursor += 2;
            let raw = &vectors[cursor..cursor + size];
            cursor += size;
            let count = vectors[cursor];
            cursor += 1;
            let actual = decode(charset, raw);
            if count == 255 {
                assert!(actual.is_err(), "{charset:?} {raw:02x?}");
            } else {
                let mut expected = String::new();
                for _ in 0..count {
                    let value = u32::from_le_bytes(vectors[cursor..cursor + 4].try_into().unwrap());
                    expected.push(char::from_u32(value).unwrap());
                    cursor += 4;
                }
                assert_eq!(
                    actual.as_deref(),
                    Ok(expected.as_str()),
                    "{charset:?} {raw:02x?}"
                );
            }
            cases += 1;
        }
        assert_eq!(cases, 197120);
    }

    #[test]
    fn utf8_preserves_both_wave_characters_and_bom() {
        let source = "\u{feff}〜～−－¢￠£￡¬￢∥‖\\~😀";
        assert_eq!(
            decode(Charset::Utf8, source.as_bytes()).as_deref(),
            Ok(source)
        );
        for raw in [vec![0xff], vec![0xc0, 0xaf], vec![0xed, 0xa0, 0x80]] {
            assert!(decode(Charset::Utf8, &raw).is_err());
        }
    }

    #[test]
    fn mixed_ascii_kana_and_jis_characters() {
        assert_eq!(
            decode(Charset::Sjis, b"a\\~\xb1\x81\x60z").as_deref(),
            Ok("a\\~ｱ〜z")
        );
        assert_eq!(
            decode(Charset::EucJp, b"a\\~\x8e\xb1\xa1\xc1z").as_deref(),
            Ok("a\\~ｱ〜z")
        );
    }
}
