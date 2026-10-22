#![forbid(unsafe_code)]

use rusqlite::{Connection, params_from_iter, types::Value};
use std::collections::BTreeMap;
#[cfg(test)]
use std::{error::Error, io};

pub const PAGE_SIZE: i64 = 50;

#[derive(Clone, Copy, Debug)]
pub enum SortMode {
    Score,
    Newcome,
}

impl SortMode {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "score" => Ok(Self::Score),
            "newcome" => Ok(Self::Newcome),
            _ => Err("sort must be score or newcome"),
        }
    }

    fn order_by(self) -> &'static str {
        match self {
            Self::Score => "ORDER BY score DESC",
            Self::Newcome => "ORDER BY score_id DESC",
        }
    }
}

pub fn page_offset(start: &str) -> Result<i64, &'static str> {
    if start.is_empty() || !start.bytes().all(|c| c.is_ascii_digit()) {
        return Err("start must be a nonnegative decimal integer");
    }
    let start: i64 = start.parse().map_err(|_| "start is too large")?;
    Ok(start / PAGE_SIZE * PAGE_SIZE)
}

#[derive(Default)]
pub struct Filters {
    clauses: Vec<String>,
    values: Vec<Value>,
}

impl Filters {
    fn add(&mut self, clause: &str, value: Value) {
        self.clauses.push(clause.to_owned());
        self.values.push(value);
    }

    fn where_sql(&self) -> String {
        if self.clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", self.clauses.join(" AND "))
        }
    }

    pub fn parse(args: Vec<String>) -> Result<Self, &'static str> {
        const ALLOWED: &[&str] = &[
            "last_days",
            "fd",
            "td",
            "race_id",
            "class_id",
            "personality_id",
            "realm1_id",
            "realm2_id",
            "name",
            "name_match",
            "sex",
            "killer",
        ];
        if args.len() > ALLOWED.len() {
            return Err("too many filters");
        }
        let mut input = BTreeMap::new();
        for arg in args {
            let (key, value) = arg.split_once('=').ok_or("filters must be key=value")?;
            if !ALLOWED.contains(&key) || value.len() > 4096 {
                return Err("unknown filter or filter value exceeds 4096 bytes");
            }
            if input.insert(key.to_owned(), value.to_owned()).is_some() {
                return Err("duplicate filter");
            }
        }
        let text = |key: &str| input.get(key).map(String::as_str).unwrap_or("");
        let number = |key: &str| -> Result<Option<i64>, &'static str> {
            let value = text(key);
            if value.is_empty() {
                Ok(None)
            } else {
                value
                    .parse()
                    .map(Some)
                    .map_err(|_| "invalid integer filter")
            }
        };
        let mut result = Self::default();
        if let Some(days) = number("last_days")?.filter(|days| *days > 0) {
            result.add(
                "date >= datetime('now', 'localtime', ?)",
                Value::Text(format!("-{days} days")),
            );
        }
        if !text("fd").is_empty() && !text("td").is_empty() {
            result.add(
                "(date >= date(?) AND date < date(?, '+1 days'))",
                Value::Text(text("fd").into()),
            );
            result.values.push(Value::Text(text("td").into()));
        }
        // Column names are selected from this constant list, never from input.
        for key in ["race_id", "class_id", "personality_id"] {
            if let Some(value) = number(key)?.filter(|value| *value > 0) {
                result.add(&format!("{key} = ?"), Value::Integer(value));
            }
        }
        for (key, seq) in [("realm1_id", 1), ("realm2_id", 2)] {
            if let Some(value) = number(key)?.filter(|value| *value > 0) {
                result.add(
                    &format!("score_id IN (SELECT score_id FROM score_realms WHERE realm_id = ? AND realm_seq = {seq})"),
                    Value::Integer(value),
                );
            }
        }
        if !text("name").is_empty() {
            if text("name_match") == "strict" {
                result.add("name = ?", Value::Text(text("name").into()));
            } else {
                // Preserve legacy SQL LIKE semantics, including % and _.
                result.add("name LIKE ?", Value::Text(format!("%{}%", text("name"))));
            }
        }
        if let Some(sex) = number("sex")? {
            result.add("sex = ?", Value::Integer(sex));
        }
        if !text("killer").is_empty() {
            result.add(
                "(killer LIKE ? OR killer = ? OR killer = ?)",
                Value::Text(format!("%{}%", text("killer"))),
            );
            result
                .values
                .push(Value::Text(format!("麻痺状態で{}", text("killer"))));
            result
                .values
                .push(Value::Text(format!("彫像状態で{}", text("killer"))));
        }
        Ok(result)
    }
}

pub struct SearchResult {
    pub total: i64,
    pub rows: Vec<(i64, i64)>,
}

pub fn search(
    conn: &Connection,
    mode: SortMode,
    offset: i64,
    filters: &Filters,
) -> rusqlite::Result<SearchResult> {
    // Keep the legacy joins, LIMIT placement and ordering. In particular, do
    // not silently add a score_id tie-breaker or move LIMIT after the joins.
    // Only fixed enum-derived SQL is interpolated; values are bound parameters.
    let order_by = mode.order_by();
    let where_sql = filters.where_sql();
    let sql = format!(
        "WITH target_score_ids AS (
            SELECT score_id FROM scores {where_sql} {order_by} LIMIT ?, ?
         )
         SELECT *,
           CASE
             WHEN killer = 'ripe' THEN '勝利の後引退'
             WHEN killer = 'Seppuku' THEN '勝利の後切腹'
             WHEN dead_place IS NULL THEN killer || 'に殺された'
             ELSE dead_place || 'で' || killer || 'に殺された'
           END AS death_reason
         FROM (
           SELECT score_id, group_concat(realm_name) AS realms_name
           FROM target_score_ids
           NATURAL LEFT JOIN score_realms
           NATURAL LEFT JOIN realms
           GROUP BY score_id
         )
         NATURAL INNER JOIN scores
         NATURAL INNER JOIN races
         NATURAL INNER JOIN classes
         NATURAL INNER JOIN personalities
         {order_by}"
    );
    let mut values = filters.values.clone();
    values.extend([Value::Integer(offset), Value::Integer(PAGE_SIZE)]);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values.iter()), |row| {
        Ok((row.get("score_id")?, row.get("score")?))
    })?;
    let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let total = conn.query_row(
        &format!("SELECT COUNT(*) FROM scores {where_sql}"),
        params_from_iter(filters.values.iter()),
        |row| row.get(0),
    )?;
    Ok(SearchResult { total, rows })
}

#[cfg(test)]
fn ranking(conn: &Connection, mode: SortMode, offset: i64) -> rusqlite::Result<Vec<(i64, i64)>> {
    Ok(search(conn, mode, offset, &Filters::default())?.rows)
}

#[cfg(test)]
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn fixture() -> rusqlite::Result<Connection> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE scores (
               score_id INTEGER PRIMARY KEY, score INTEGER NOT NULL,
               killer TEXT, dead_place TEXT,
               race_id INTEGER, class_id INTEGER, personality_id INTEGER,
               name TEXT, sex INTEGER, date TEXT
             );
             CREATE TABLE races (race_id INTEGER PRIMARY KEY, race_name TEXT);
             CREATE TABLE classes (class_id INTEGER PRIMARY KEY, class_name TEXT);
             CREATE TABLE personalities (personality_id INTEGER PRIMARY KEY, personality_name TEXT);
             CREATE TABLE realms (realm_id INTEGER PRIMARY KEY, realm_name TEXT);
             CREATE TABLE score_realms (score_id INTEGER, realm_id INTEGER, realm_seq INTEGER);
             INSERT INTO races VALUES (1, '人間');
             INSERT INTO classes VALUES (1, '戦士');
             INSERT INTO personalities VALUES (1, 'ふつう');
             INSERT INTO realms VALUES (1, '生命'), (2, '自然');",
        )?;
        for id in 0..55_i64 {
            conn.execute(
                "INSERT INTO scores VALUES (?1, ?2, 'ripe', NULL, 1, 1, 1, 'テスト', 0, '2026-10-19 17:32:45')",
                params![id, 1000 - id],
            )?;
        }
        Ok(conn)
    }

    #[test]
    fn score_order_and_page_limit() -> rusqlite::Result<()> {
        let rows = ranking(&fixture()?, SortMode::Score, 0)?;
        assert_eq!(rows.len(), 50);
        assert_eq!(rows.first(), Some(&(0, 1000)));
        assert_eq!(rows.last(), Some(&(49, 951)));
        Ok(())
    }

    #[test]
    fn newcome_orders_by_id() -> rusqlite::Result<()> {
        let rows = ranking(&fixture()?, SortMode::Newcome, 0)?;
        assert_eq!(rows.len(), 50);
        assert_eq!(rows.first(), Some(&(54, 946)));
        assert_eq!(rows.last(), Some(&(5, 995)));
        Ok(())
    }

    #[test]
    fn last_and_out_of_range_pages() -> rusqlite::Result<()> {
        let conn = fixture()?;
        assert_eq!(ranking(&conn, SortMode::Score, 50)?.len(), 5);
        assert!(ranking(&conn, SortMode::Score, 100)?.is_empty());
        conn.execute("DELETE FROM scores", [])?;
        assert!(ranking(&conn, SortMode::Score, 0)?.is_empty());
        Ok(())
    }

    #[test]
    fn realms_do_not_duplicate_scores() -> rusqlite::Result<()> {
        let conn = fixture()?;
        conn.execute_batch("INSERT INTO score_realms VALUES (0, 1, 1), (0, 2, 2);")?;
        let rows = ranking(&conn, SortMode::Score, 0)?;
        assert_eq!(rows.len(), 50);
        assert_eq!(rows.iter().filter(|(id, _)| *id == 0).count(), 1);
        Ok(())
    }

    #[test]
    fn ties_keep_both_scores_without_promising_order() -> rusqlite::Result<()> {
        let conn = fixture()?;
        conn.execute("UPDATE scores SET score = 1000 WHERE score_id = 1", [])?;
        let rows = ranking(&conn, SortMode::Score, 0)?;
        let mut top = rows[..2].to_vec();
        top.sort_unstable();
        assert_eq!(top, vec![(0, 1000), (1, 1000)]);
        Ok(())
    }

    #[test]
    fn paging_and_invalid_input() {
        for (input, offset) in [("0", 0), ("49", 0), ("50", 50), ("51", 50)] {
            assert_eq!(page_offset(input), Ok(offset));
        }
        for input in ["", "-1", "abc", "1.5", "9223372036854775808"] {
            assert!(page_offset(input).is_err());
        }
        assert!(SortMode::parse("score DESC; DROP TABLE scores").is_err());
    }

    fn filtered(conn: &Connection, args: &[&str]) -> Result<SearchResult, Box<dyn Error>> {
        let filters =
            Filters::parse(args.iter().map(|arg| (*arg).to_owned()).collect()).map_err(invalid)?;
        Ok(search(conn, SortMode::Score, 0, &filters)?)
    }

    #[test]
    fn zero_sex_and_combined_filters() -> Result<(), Box<dyn Error>> {
        let conn = fixture()?;
        let result = filtered(
            &conn,
            &["sex=0", "race_id=1", "class_id=1", "personality_id=1"],
        )?;
        assert_eq!(result.total, 55);
        assert_eq!(result.rows.len(), 50);
        assert_eq!(filtered(&conn, &["sex=1"])?.total, 0);
        assert_eq!(filtered(&conn, &["race_id=0"])?.total, 55);
        Ok(())
    }

    #[test]
    fn realm_sequence_is_respected() -> Result<(), Box<dyn Error>> {
        let conn = fixture()?;
        conn.execute_batch("INSERT INTO score_realms VALUES (0, 1, 1), (0, 2, 2), (1, 2, 1);")?;
        assert_eq!(
            filtered(&conn, &["realm1_id=1", "realm2_id=2"])?.rows,
            vec![(0, 1000)]
        );
        assert_eq!(filtered(&conn, &["realm1_id=2"])?.rows, vec![(1, 999)]);
        assert_eq!(filtered(&conn, &["realm2_id=1"])?.total, 0);
        Ok(())
    }

    #[test]
    fn name_matching_and_sql_injection() -> Result<(), Box<dyn Error>> {
        let conn = fixture()?;
        conn.execute(
            "UPDATE scores SET name = '勇者あいう' WHERE score_id = 0",
            [],
        )?;
        assert_eq!(
            filtered(&conn, &["name=勇者", "name_match=partial"])?.rows,
            vec![(0, 1000)]
        );
        assert_eq!(
            filtered(&conn, &["name=勇者", "name_match=strict"])?.total,
            0
        );
        assert_eq!(
            filtered(&conn, &["name=' OR 1=1 --", "name_match=strict"])?.total,
            0
        );
        assert_eq!(filtered(&conn, &["name=%"])?.total, 55);
        Ok(())
    }

    #[test]
    fn dates_and_killer() -> Result<(), Box<dyn Error>> {
        let conn = fixture()?;
        assert_eq!(
            filtered(&conn, &["fd=2026-10-19", "td=2026-10-19", "killer=ripe"])?.total,
            55
        );
        assert_eq!(
            filtered(&conn, &["fd=2026-10-20", "td=2026-10-21"])?.total,
            0
        );
        assert_eq!(filtered(&conn, &["fd=2026-10-20"])?.total, 55);
        assert_eq!(filtered(&conn, &["killer=存在しない死因"])?.total, 0);
        Ok(())
    }

    #[test]
    fn filter_keys_duplicates_sizes_and_numbers_are_checked() {
        for args in [
            vec!["unknown=1"],
            vec!["sex=0", "sex=1"],
            vec!["race_id=abc"],
            vec!["sex"],
            vec!["sex=9223372036854775808"],
        ] {
            assert!(Filters::parse(args.into_iter().map(str::to_owned).collect()).is_err());
        }
        assert!(Filters::parse(vec![format!("name={}", "x".repeat(4097))]).is_err());
    }
}
