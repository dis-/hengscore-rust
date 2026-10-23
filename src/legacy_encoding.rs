//! Valid-input compatibility with PHP libmbfl SJIS / EUC-JP.
//! JIS tables are derived from PHP; see php-reference/NOTICE and LICENSE.
//! `decode` rejects invalid sequences. `decode_lossy` replaces them with `?`,
//! the default `mb_substitute_character` of the legacy PHP server, and exists
//! only for the screen shot section of a submission.

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
const SUBSTITUTE: char = '?';

fn lookup(table: &[u8], row: usize, column: usize) -> Option<char> {
    let index = (row * 94 + column) * 2;
    let pair = table.get(index..index + 2)?;
    let value = u16::from_le_bytes([pair[0], pair[1]]);
    if value == 0 {
        return None;
    }
    char::from_u32(u32::from(value))
}

/// Decodes the Shift_JIS character whose first byte is `first` and whose
/// remaining bytes start `tail`. Returns the character (`None` for an invalid
/// sequence) and how many bytes of `tail` belong to it. An invalid sequence
/// never claims a following ASCII byte, so substituting it cannot swallow a
/// newline or HTML markup.
fn sjis_step(first: u8, tail: &[u8]) -> (Option<char>, usize) {
    if first.is_ascii() {
        return (Some(char::from(first)), 0);
    }
    if (0xa1..=0xdf).contains(&first) {
        return (char::from_u32(0xfec0 + u32::from(first)), 0);
    }
    if !((0x81..=0x9f).contains(&first) || (0xe0..=0xef).contains(&first)) {
        return (None, 0);
    }
    let Some(&second) = tail.first() else {
        return (None, 0);
    };
    if !(0x40..=0xfc).contains(&second) || second == 0x7f {
        return (None, usize::from(!second.is_ascii()));
    }
    let lead = usize::from(first - if first < 0xa0 { 0x81 } else { 0xc1 });
    let trail = usize::from(second - if second < 0x7f { 0x40 } else { 0x41 });
    (lookup(JIS0208, lead * 2 + trail / 94, trail % 94), 1)
}

/// EUC-JP counterpart of `sjis_step`.
fn eucjp_step(first: u8, tail: &[u8]) -> (Option<char>, usize) {
    if first.is_ascii() {
        return (Some(char::from(first)), 0);
    }
    let Some(&second) = tail.first() else {
        return (None, 0);
    };
    let graphic = |byte: u8| (0xa1..=0xfe).contains(&byte);
    match first {
        0x8e if (0xa1..=0xdf).contains(&second) => (char::from_u32(0xfec0 + u32::from(second)), 1),
        0x8f if graphic(second) => {
            let Some(&third) = tail.get(1) else {
                return (None, 1);
            };
            if !graphic(third) {
                return (None, 1 + usize::from(!third.is_ascii()));
            }
            (
                lookup(
                    JIS0212,
                    usize::from(second - 0xa1),
                    usize::from(third - 0xa1),
                ),
                2,
            )
        }
        0xa1..=0xfe if graphic(second) => (
            lookup(
                JIS0208,
                usize::from(first - 0xa1),
                usize::from(second - 0xa1),
            ),
            1,
        ),
        // A lead byte followed by a byte that cannot continue it.
        0x8e | 0x8f | 0xa1..=0xfe => (None, usize::from(!second.is_ascii())),
        _ => (None, 0),
    }
}

/// Splits `raw` into characters with `step`; `None` marks an invalid sequence.
fn legacy_chars(
    step: fn(u8, &[u8]) -> (Option<char>, usize),
    raw: &[u8],
) -> impl Iterator<Item = Option<char>> {
    let mut index = 0;
    std::iter::from_fn(move || {
        let &first = raw.get(index)?;
        let (decoded, extra) = step(first, &raw[index + 1..]);
        index += 1 + extra;
        Some(decoded)
    })
}

pub fn decode(charset: Charset, raw: &[u8]) -> Result<String, &'static str> {
    match charset {
        Charset::Ascii if !raw.is_ascii() => Err(BAD),
        Charset::Ascii | Charset::Utf8 => {
            std::str::from_utf8(raw).map(str::to_owned).map_err(|_| BAD)
        }
        Charset::Sjis => legacy_chars(sjis_step, raw)
            .map(|decoded| decoded.ok_or(BAD))
            .collect(),
        Charset::EucJp => legacy_chars(eucjp_step, raw)
            .map(|decoded| decoded.ok_or(BAD))
            .collect(),
    }
}

/// Decodes like `decode`, but replaces every invalid sequence with `?`.
pub fn decode_lossy(charset: Charset, raw: &[u8]) -> String {
    match charset {
        Charset::Ascii => raw
            .iter()
            .map(|&byte| {
                if byte.is_ascii() {
                    char::from(byte)
                } else {
                    SUBSTITUTE
                }
            })
            .collect(),
        Charset::Utf8 => {
            let mut output = String::with_capacity(raw.len());
            for chunk in raw.utf8_chunks() {
                output.push_str(chunk.valid());
                if !chunk.invalid().is_empty() {
                    output.push(SUBSTITUTE);
                }
            }
            output
        }
        Charset::Sjis => legacy_chars(sjis_step, raw)
            .map(|decoded| decoded.unwrap_or(SUBSTITUTE))
            .collect(),
        Charset::EucJp => legacy_chars(eucjp_step, raw)
            .map(|decoded| decoded.unwrap_or(SUBSTITUTE))
            .collect(),
    }
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
            assert_eq!(decode_lossy(Charset::Ascii, &[b'a', byte, b'z']), "a?z");
        }
    }

    /// Every case of php-reference/compat_vectors.bin: the charset, the raw
    /// bytes and the PHP decoder's text, or `None` where PHP found the input
    /// invalid. Produced by compiling the original PHP C decoder functions, not
    /// by the Rust decoder or Python codecs. Includes all 1/2-byte strings and
    /// all 0x8f-prefixed 3-byte EUC-JP strings, including invalid sequences.
    fn php_vectors() -> Vec<(Charset, &'static [u8], Option<String>)> {
        let vectors = include_bytes!("../php-reference/compat_vectors.bin");
        let mut cursor = 0;
        let mut cases = Vec::new();
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
            let expected = if count == 255 {
                None
            } else {
                let mut text = String::new();
                for _ in 0..count {
                    let value = u32::from_le_bytes(vectors[cursor..cursor + 4].try_into().unwrap());
                    text.push(char::from_u32(value).unwrap());
                    cursor += 4;
                }
                Some(text)
            };
            cases.push((charset, raw, expected));
        }
        assert_eq!(cases.len(), 197120);
        cases
    }

    #[test]
    fn php_c_decoder_exhaustive_vectors() {
        for (charset, raw, expected) in php_vectors() {
            assert_eq!(
                decode(charset, raw).ok(),
                expected,
                "{charset:?} {raw:02x?}"
            );
        }
    }

    #[test]
    fn lossy_decoding_substitutes_exactly_what_php_rejects() {
        for (charset, raw, expected) in php_vectors() {
            let lossy = decode_lossy(charset, raw);
            match expected {
                Some(text) => assert_eq!(lossy, text, "{charset:?} {raw:02x?}"),
                None => assert!(lossy.contains(SUBSTITUTE), "{charset:?} {raw:02x?}"),
            }
        }
    }

    #[test]
    fn lossy_decoding_keeps_the_ascii_byte_after_a_broken_sequence() {
        // A lead byte without its second byte before markup and at the end.
        for (charset, raw) in [
            (Charset::Sjis, &b"\x82</font>\x82"[..]),
            (Charset::Sjis, b"\x80</font>\xa0"),
            (Charset::EucJp, b"\xa4</font>\xa4"),
            (Charset::EucJp, b"\x8e</font>\x8f"),
            (Charset::EucJp, b"\x8f\xa2</font>\x8f\xa2"),
            (Charset::Utf8, b"\xe3\x82</font>\xe3"),
            (Charset::Ascii, b"\x82</font>\x82"),
        ] {
            assert!(decode(charset, raw).is_err(), "{charset:?}");
            assert_eq!(decode_lossy(charset, raw), "?</font>?", "{charset:?}");
        }
        // A non-ASCII byte that cannot continue the sequence is part of it.
        for (charset, raw) in [
            (Charset::Sjis, &b"\x82\xfdz"[..]),
            (Charset::EucJp, b"\xa4\x80z"),
            (Charset::EucJp, b"\x8e\xe0z"),
            (Charset::EucJp, b"\x8f\x80z"),
            (Charset::EucJp, b"\x8f\xa2\x80z"),
        ] {
            assert_eq!(decode_lossy(charset, raw), "?z", "{charset:?}");
        }
        assert_eq!(decode_lossy(Charset::Utf8, b"\xe3\x82\xfez"), "??z");
    }

    #[test]
    fn utf8_preserves_both_wave_characters_and_bom() {
        let source = "\u{feff}〜～−－¢￠£￡¬￢∥‖\\~😀";
        assert_eq!(
            decode(Charset::Utf8, source.as_bytes()).as_deref(),
            Ok(source)
        );
        assert_eq!(decode_lossy(Charset::Utf8, source.as_bytes()), source);
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
