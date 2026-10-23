#![forbid(unsafe_code)]
use hengscore::ranking::{SortMode, page_offset, ranking};
use rusqlite::{Connection, OpenFlags};
use std::{env, error::Error, io, path::Path};
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let db = args
        .next()
        .ok_or_else(|| invalid("usage: ranking_probe DB score|newcome START"))?;
    let mode = args.next().ok_or_else(|| invalid("missing sort"))?;
    let start = args.next().ok_or_else(|| invalid("missing start"))?;
    if args.next().is_some() {
        return Err(invalid("too many arguments").into());
    }
    let mode =
        SortMode::parse(mode.to_str().ok_or_else(|| invalid("invalid sort"))?).map_err(invalid)?;
    let offset =
        page_offset(start.to_str().ok_or_else(|| invalid("invalid start"))?).map_err(invalid)?;
    let conn = Connection::open_with_flags(Path::new(&db), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    for (id, score) in ranking(&conn, mode, offset)? {
        println!("{id}|{score}");
    }
    Ok(())
}
