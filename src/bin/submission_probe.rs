#![forbid(unsafe_code)]
use hengscore_poc::submission::{MAX_RAW, parse_submission};
use std::{
    env,
    error::Error,
    ffi::OsStr,
    fs::File,
    io::{self, Read, Write},
};
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn hex_line(out: &mut impl Write, label: &str, value: &str) -> io::Result<()> {
    write!(out, "{label}\t")?;
    for byte in value.as_bytes() {
        write!(out, "{byte:02x}")?;
    }
    writeln!(out)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let header = args
        .next()
        .ok_or_else(|| invalid("usage: submission_probe CONTENT_TYPE FILE [--details]"))?;
    let path = args.next().ok_or_else(|| invalid("missing input file"))?;
    let details = match args.next() {
        None => false,
        Some(flag) if flag == OsStr::new("--details") => true,
        Some(_) => return Err(invalid("unknown argument").into()),
    };
    if args.next().is_some() {
        return Err(invalid("too many arguments").into());
    }
    let header = header
        .to_str()
        .ok_or_else(|| invalid("invalid content type"))?;
    let input: Box<dyn Read> = if path == OsStr::new("-") {
        Box::new(io::stdin())
    } else {
        Box::new(File::open(path)?)
    };
    let mut raw = Vec::new();
    input.take((MAX_RAW + 1) as u64).read_to_end(&mut raw)?;
    let parsed = parse_submission(header, &raw).map_err(invalid)?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    writeln!(out, "OK")?;
    writeln!(out, "INFO_FIELDS={}", parsed.info.len())?;
    writeln!(out, "DUMP_LINES={}", parsed.dump_lines.len())?;
    match &parsed.screen_lines {
        Some(lines) => writeln!(out, "SCREEN_LINES={}", lines.len())?,
        None => writeln!(out, "SCREEN_LINES=none")?,
    }
    if details {
        // Hex output is used by synthetic-fixture tests, never interpreted as HTML.
        for (key, value) in &parsed.info {
            hex_line(&mut out, &format!("FIELD\t{key}"), value)?;
        }
        for line in &parsed.dump_lines {
            hex_line(&mut out, "DUMP", line)?;
        }
        if let Some(lines) = &parsed.screen_lines {
            for line in lines {
                hex_line(&mut out, "SCREEN", line)?;
            }
        }
    }
    out.flush()?;
    Ok(())
}
