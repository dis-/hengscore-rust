use crate::legacy_encoding::{Charset, decode, decode_lossy};
use std::collections::BTreeMap;

pub const MAX_RAW: usize = 8 * 1024 * 1024;
const MAX_DECODED: usize = 24 * 1024 * 1024;
const MAX_LINES: usize = 100_000;
const MAX_LINE: usize = 256 * 1024;
const MAX_INFO_LINES: usize = 64;
const MAX_VALUE: usize = 4096;
const DUMP_MARKER: &str = "-----charcter dump-----";
const SCREEN_MARKER: &str = "-----screen shot-----";
const REQUIRED: &[&str] = &[
    "name", "version", "score", "level", "depth", "maxlv", "maxdp", "au", "turns", "sex", "race",
    "class", "seikaku", "realm1", "realm2", "killer",
];

#[derive(Debug)]
pub struct Submission {
    pub ascii: bool,
    pub info: BTreeMap<String, String>,
    pub dump_lines: Vec<String>,
    // Untrusted HTML, with `?` in place of malformed bytes. Parsing it does NOT
    // authorize rendering or publishing it.
    pub screen_lines: Option<Vec<String>>,
}

fn php_trim(value: &str) -> &str {
    value.trim_matches([' ', '\t', '\n', '\r', '\0', '\u{000b}'])
}

pub fn validate_content_type(content_type: &str) -> Result<(), &'static str> {
    encoding(content_type).map(|_| ())
}

fn encoding(content_type: &str) -> Result<Charset, &'static str> {
    if content_type.len() > 256 {
        return Err("content type too long");
    }
    let mut parts = content_type.split(';');
    if parts.next() != Some("text/plain") {
        return Err("unsupported media type");
    }
    let parameter = parts.next().ok_or("missing charset")?;
    if parts.next().is_some() {
        return Err("extra content type parameter");
    }
    let charset = php_trim(parameter)
        .strip_prefix("charset=")
        .ok_or("missing charset")?;
    match php_trim(charset).to_ascii_lowercase().as_str() {
        "ascii" | "us-ascii" => Ok(Charset::Ascii),
        "utf-8" => Ok(Charset::Utf8),
        "euc-jp" => Ok(Charset::EucJp),
        "shift_jis" => Ok(Charset::Sjis),
        _ => Err("unsupported charset"),
    }
}

/// Decodes one section line by line, strictly or with `?` substitution, while
/// enforcing the line length limit and the shared decoded size budget.
fn decode_section(
    charset: Charset,
    lines: &[&[u8]],
    lossy: bool,
    budget: &mut usize,
) -> Result<Vec<String>, &'static str> {
    lines
        .iter()
        .map(|line| {
            let text = if lossy {
                decode_lossy(charset, line)
            } else {
                decode(charset, line)?
            };
            if text.len() > MAX_LINE {
                return Err("line length limit exceeded");
            }
            *budget = budget
                .checked_sub(text.len())
                .ok_or("decoded body limit exceeded")?;
            Ok(text)
        })
        .collect()
}

pub fn parse_submission(content_type: &str, raw: &[u8]) -> Result<Submission, &'static str> {
    if raw.len() > MAX_RAW {
        return Err("raw body limit exceeded");
    }
    let charset = encoding(content_type)?;
    // NUL is a single ASCII byte in every supported charset, so checking the
    // raw body equals checking the decoded text.
    if raw.contains(&0) {
        return Err("NUL byte is not allowed");
    }
    // Split and locate the markers before decoding: LF never continues a
    // multibyte sequence in any supported charset and the markers are ASCII,
    // so the sections are found even when the screen shot holds broken bytes.
    // Bound the line index allocation, including input containing only newlines.
    let lines: Vec<&[u8]> = raw
        .split(|&byte| byte == b'\n')
        .take(MAX_LINES + 1)
        .collect();
    if lines.len() > MAX_LINES {
        return Err("line count limit exceeded");
    }
    let dump_at = lines
        .iter()
        .position(|line| *line == DUMP_MARKER.as_bytes())
        .ok_or("missing dump marker")?;
    let screen_at = lines
        .iter()
        .position(|line| *line == SCREEN_MARKER.as_bytes());
    if screen_at.is_some_and(|index| index < dump_at) {
        return Err("screen marker precedes dump marker");
    }
    if dump_at > MAX_INFO_LINES {
        return Err("metadata line limit exceeded");
    }
    let mut budget = MAX_DECODED;
    let mut info = BTreeMap::new();
    for line in decode_section(charset, &lines[..dump_at], false, &mut budget)? {
        // Legacy PHP ignores header lines with no colon.
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.is_empty()
            || key.len() > 64
            || !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err("invalid metadata key");
        }
        let value = php_trim(value);
        if value.len() > MAX_VALUE {
            return Err("metadata value limit exceeded");
        }
        // Intentional stricter behavior: PHP silently accepts the last duplicate.
        if info.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err("duplicate metadata key");
        }
    }
    for key in REQUIRED {
        if !info.contains_key(*key) {
            return Err("missing required field");
        }
    }
    for key in [
        "score", "level", "depth", "maxlv", "maxdp", "au", "turns", "sex",
    ] {
        let number: i64 = info
            .get(key)
            .ok_or("missing numeric field")?
            .parse()
            .map_err(|_| "invalid or overflowing integer")?;
        // Historic data contains negative scores and levels above 50.
        if key != "score" && number < 0 {
            return Err("negative non-score value");
        }
        if (key == "level" || key == "maxlv") && number == 0 {
            return Err("level must be positive");
        }
        if key == "sex" && number > 1 {
            return Err("unsupported sex value");
        }
    }
    let dump_end = screen_at.unwrap_or(lines.len());
    let dump_lines = decode_section(charset, &lines[dump_at + 1..dump_end], false, &mut budget)?;
    if dump_lines.iter().all(String::is_empty) {
        return Err("empty character dump");
    }
    // The game writes the screen cell by cell, so a double-width character can
    // lose its second byte (seen on a samurai's seppuku screen). Legacy PHP
    // accepted such screens with `?` in place of the broken bytes; do the same
    // rather than rejecting the whole submission.
    let screen_lines = screen_at
        .map(|index| decode_section(charset, &lines[index + 1..], true, &mut budget))
        .transpose()?;
    Ok(Submission {
        ascii: matches!(charset, Charset::Ascii),
        info,
        dump_lines,
        screen_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        "name: 勇者\nversion: 変愚蛮怒 3.0\nscore: -323\nlevel: 255\ndepth: 0\nmaxlv: 255\nmaxdp: 127\nau: 0\nturns: 10\nsex: 0\nrace: 人間\nclass: 戦士\nseikaku: ふつう\nrealm1: 魔法なし\nrealm2: 魔法なし\nkiller: ripe\n-----charcter dump-----\nキャラクターダンプ\n".into()
    }

    #[test]
    fn utf8_and_optional_screen() -> Result<(), &'static str> {
        let source = fixture();
        let parsed = parse_submission("text/plain; charset=UTF-8", source.as_bytes())?;
        assert_eq!(parsed.info["score"], "-323");
        assert_eq!(parsed.info["level"], "255");
        assert_eq!(parsed.dump_lines, vec!["キャラクターダンプ", ""]);
        assert!(parsed.screen_lines.is_none());
        let source = source + "-----screen shot-----\n<html><pre>画面</pre></html>\n";
        let parsed = parse_submission("text/plain; charset=utf-8", source.as_bytes())?;
        assert_eq!(parsed.dump_lines, vec!["キャラクターダンプ"]);
        assert_eq!(
            parsed.screen_lines,
            Some(vec!["<html><pre>画面</pre></html>".into(), String::new()])
        );
        Ok(())
    }

    /// The fixture with ASCII-only values, valid in every supported charset.
    fn ascii_fixture() -> String {
        fixture()
            .replace("勇者", "TOKEN")
            .replace("変愚蛮怒", "Hengband")
            .replace("人間", "Human")
            .replace("戦士", "Warrior")
            .replace("ふつう", "Ordinary")
            .replace("魔法なし", "None")
            .replace("キャラクターダンプ", "DUMP")
    }

    /// `source` with its first `placeholder` replaced by `bytes`.
    fn splice(source: &[u8], placeholder: &[u8], bytes: &[u8]) -> Vec<u8> {
        let at = source
            .windows(placeholder.len())
            .position(|window| window == placeholder)
            .expect("placeholder present");
        [&source[..at], bytes, &source[at + placeholder.len()..]].concat()
    }

    #[test]
    fn legacy_encodings() -> Result<(), &'static str> {
        for (charset, encoded) in [
            ("EUC-JP", &[0xa1, 0xc1][..]),
            ("SHIFT_JIS", &[0x81, 0x60][..]),
        ] {
            let bytes = splice(ascii_fixture().as_bytes(), b"TOKEN", encoded);
            let parsed = parse_submission(&format!("text/plain; charset={charset}"), &bytes)?;
            assert_eq!(parsed.info["name"], "〜");
            assert_eq!(parsed.dump_lines[0], "DUMP");
        }
        Ok(())
    }

    #[test]
    fn screen_shot_accepts_malformed_bytes_as_substitutes() -> Result<(), &'static str> {
        let source = ascii_fixture()
            + "-----screen shot-----\n<pre>WAVE<font color=\"#ffffff\">STRAY</font>STRAY\n</pre>\n";
        for (charset, wave, decoded_wave, stray) in [
            ("SHIFT_JIS", &[0x81, 0x60][..], "〜", &[0x82][..]),
            ("EUC-JP", &[0xa1, 0xc1][..], "〜", &[0xa4][..]),
            ("UTF-8", "〜".as_bytes(), "〜", &[0xe3][..]),
            ("ASCII", &b"~"[..], "~", &[0x82][..]),
        ] {
            // A lead byte without its second byte, once before markup and once
            // at the end of the line.
            let bytes = splice(source.as_bytes(), b"WAVE", wave);
            let bytes = splice(&bytes, b"STRAY", stray);
            let bytes = splice(&bytes, b"STRAY", stray);
            let parsed = parse_submission(&format!("text/plain; charset={charset}"), &bytes)?;
            assert_eq!(parsed.info["name"], "TOKEN");
            assert_eq!(parsed.dump_lines, vec!["DUMP"]);
            assert_eq!(
                parsed.screen_lines,
                Some(vec![
                    format!("<pre>{decoded_wave}<font color=\"#ffffff\">?</font>?"),
                    "</pre>".into(),
                    String::new(),
                ]),
                "{charset}"
            );
        }
        Ok(())
    }

    #[test]
    fn malformed_bytes_outside_the_screen_shot_are_rejected() {
        let source = ascii_fixture() + "-----screen shot-----\n<pre>SCREEN</pre>\n";
        for (charset, stray) in [
            ("SHIFT_JIS", &[0x82][..]),
            ("EUC-JP", &[0xa4][..]),
            ("UTF-8", &[0xe3][..]),
            ("ASCII", &[0x82][..]),
        ] {
            let header = format!("text/plain; charset={charset}");
            for placeholder in [&b"TOKEN"[..], &b"DUMP"[..]] {
                let bytes = splice(source.as_bytes(), placeholder, stray);
                assert_eq!(
                    parse_submission(&header, &bytes).err(),
                    Some("malformed encoded text"),
                    "{charset} {placeholder:?}"
                );
            }
            // The screen shot is the only lenient section.
            let bytes = splice(source.as_bytes(), b"SCREEN", stray);
            assert!(parse_submission(&header, &bytes).is_ok(), "{charset}");
        }
    }

    #[test]
    fn malformed_bytes_and_unsupported_headers() {
        for (header, body) in [
            ("text/plain; charset=utf-8", &[0xff][..]),
            ("text/plain; charset=shift_jis", &[0x82][..]),
            ("text/plain; charset=euc-jp", &[0xa4][..]),
        ] {
            assert!(parse_submission(header, body).is_err());
        }
        for header in [
            "text/html; charset=utf-8",
            "text/plain",
            "text/plain; charset=unknown",
            "text/plain; charset=utf-8; x=y",
        ] {
            assert!(parse_submission(header, fixture().as_bytes()).is_err());
        }
    }

    #[test]
    fn duplicate_missing_and_overflowing_fields() {
        let source = fixture();
        for altered in [
            source.replacen("name: 勇者", "name: 勇者\nname: 別名", 1),
            source.replace("race: 人間\n", ""),
            source.replace("score: -323", "score: 9223372036854775808"),
            source.replace("sex: 0", "sex: 2"),
            source.replace("level: 255", "level: 0"),
            source.replace("au: 0", "au: -1"),
            source.replace(DUMP_MARKER, "-----character dump-----"),
            source.replace('\n', "\r\n"),
        ] {
            assert!(parse_submission("text/plain; charset=utf-8", altered.as_bytes()).is_err());
        }
    }

    #[test]
    fn body_line_and_field_limits() {
        let huge = vec![b'x'; MAX_RAW + 1];
        assert!(parse_submission("text/plain; charset=utf-8", &huge).is_err());
        let source = fixture();
        for altered in [
            source.clone() + &"x".repeat(MAX_LINE + 1),
            source.clone() + &"\n".repeat(MAX_LINES + 1),
            source.replacen(
                "name: 勇者",
                &format!("name: {}", "x".repeat(MAX_VALUE + 1)),
                1,
            ),
            source + "\0",
        ] {
            assert!(parse_submission("text/plain; charset=utf-8", altered.as_bytes()).is_err());
        }
    }

    #[test]
    fn ascii_trim_preserves_ideographic_spaces_and_colons() -> Result<(), &'static str> {
        let source = fixture().replacen("name: 勇者", "name: 　勇者:太郎　 \t", 1);
        let parsed = parse_submission("text/plain; charset=utf-8", source.as_bytes())?;
        assert_eq!(parsed.info["name"], "　勇者:太郎　");
        Ok(())
    }

    #[test]
    fn unknown_fields_are_bounded_but_preserved() -> Result<(), &'static str> {
        let source = fixture().replacen(
            "name: 勇者",
            "future_field: value\nignored line\nname: 勇者",
            1,
        );
        let parsed = parse_submission("text/plain; charset=utf-8", source.as_bytes())?;
        assert_eq!(parsed.info["future_field"], "value");
        Ok(())
    }
}
