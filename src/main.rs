#![forbid(unsafe_code)]

use rusqlite::{Connection, OpenFlags};
use std::{env, error::Error, path::Path};

#[derive(Debug, PartialEq, Eq)]
struct Summary {
    score_count: i64,
    min_id: Option<i64>,
    max_id: Option<i64>,
    oldest_date: Option<String>,
    newest_date: Option<String>,
}

fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

fn summarize(conn: &Connection) -> rusqlite::Result<Summary> {
    conn.query_row(
        "SELECT COUNT(*), MIN(score_id), MAX(score_id),
                MIN(date), MAX(date)
         FROM scores",
        [],
        |row| {
            Ok(Summary {
                score_count: row.get(0)?,
                min_id: row.get(1)?,
                max_id: row.get(2)?,
                oldest_date: row.get(3)?,
                newest_date: row.get(4)?,
            })
        },
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let path = args.next().ok_or("使い方: hengscore_poc DBファイル")?;
    if args.next().is_some() {
        return Err("DBファイルを1つだけ指定してください".into());
    }

    let conn = open_read_only(Path::new(&path))?;
    let summary = summarize(&conn)?;
    println!("{summary:#?}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> rusqlite::Result<Connection> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE scores (
                score_id INTEGER PRIMARY KEY,
                date TEXT NOT NULL
             );",
        )?;
        Ok(conn)
    }

    #[test]
    fn summarizes_gaps_and_dates() -> rusqlite::Result<()> {
        let conn = fixture()?;
        conn.execute_batch(
            "INSERT INTO scores VALUES
                (0, '2000-07-02'),
                (9, '2026-10-19 17:32:45');",
        )?;

        assert_eq!(
            summarize(&conn)?,
            Summary {
                score_count: 2,
                min_id: Some(0),
                max_id: Some(9),
                oldest_date: Some("2000-07-02".into()),
                newest_date: Some("2026-10-19 17:32:45".into()),
            }
        );
        Ok(())
    }

    #[test]
    fn empty_table_is_supported() -> rusqlite::Result<()> {
        let conn = fixture()?;
        assert_eq!(
            summarize(&conn)?,
            Summary {
                score_count: 0,
                min_id: None,
                max_id: None,
                oldest_date: None,
                newest_date: None,
            }
        );
        Ok(())
    }

    #[test]
    fn read_only_rejects_writes() -> Result<(), Box<dyn Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let dir = env::temp_dir().join(format!("hengscore-poc-{}-{stamp}", std::process::id()));
        fs::create_dir(&dir)?;
        let path = dir.join("test.db");

        {
            let conn = Connection::open(&path)?;
            conn.execute_batch("CREATE TABLE scores (score_id INTEGER);")?;
        }

        let conn = open_read_only(&path)?;
        let result = conn.execute("INSERT INTO scores VALUES (1)", []);
        let rejected = matches!(
            result,
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ReadOnly
        );

        drop(conn);
        fs::remove_file(&path)?;
        fs::remove_dir(&dir)?;
        assert!(rejected, "読み取り専用接続が書き込みを拒否しませんでした");
        Ok(())
    }
}
