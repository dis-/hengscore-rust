use hengscore::search::{Filters, SortMode, page_offset, search};
use rusqlite::{Connection, OpenFlags};
use std::{env, error::Error, io, path::Path};
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let db = args
        .next()
        .ok_or_else(|| invalid("usage: ranking_filters DB score|newcome START [key=value ...]"))?;
    let mode = args.next().ok_or_else(|| invalid("missing sort"))?;
    let start = args.next().ok_or_else(|| invalid("missing start"))?;
    let filters = args
        .map(|arg| {
            arg.into_string()
                .map_err(|_| invalid("invalid filter encoding"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let filters = Filters::parse(filters).map_err(invalid)?;
    let mode =
        SortMode::parse(mode.to_str().ok_or_else(|| invalid("invalid sort"))?).map_err(invalid)?;
    let offset =
        page_offset(start.to_str().ok_or_else(|| invalid("invalid start"))?).map_err(invalid)?;
    let conn = Connection::open_with_flags(Path::new(&db), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.execute_batch("BEGIN")?;
    let result = search(&conn, mode, offset, &filters)?;
    conn.execute_batch("COMMIT")?;
    eprintln!("TOTAL_MATCHES={}", result.total);
    for (id, score) in result.rows {
        println!("{id}|{score}");
    }
    Ok(())
}
