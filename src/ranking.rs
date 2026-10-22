use rusqlite::{Connection, params};

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

pub fn ranking(
    conn: &Connection,
    mode: SortMode,
    offset: i64,
) -> rusqlite::Result<Vec<(i64, i64)>> {
    // Keep the legacy joins, LIMIT placement and ordering. In particular, do
    // not silently add a score_id tie-breaker or move LIMIT after the joins.
    // Only fixed enum-derived SQL is interpolated; values are bound parameters.
    let order_by = mode.order_by();
    let sql = format!(
        "WITH target_score_ids AS (
            SELECT score_id FROM scores {order_by} LIMIT ?1, ?2
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
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![offset, PAGE_SIZE], |row| {
        Ok((row.get("score_id")?, row.get("score")?))
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> rusqlite::Result<Connection> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE scores (
               score_id INTEGER PRIMARY KEY, score INTEGER NOT NULL,
               killer TEXT, dead_place TEXT,
               race_id INTEGER, class_id INTEGER, personality_id INTEGER
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
                "INSERT INTO scores VALUES (?1, ?2, 'ripe', NULL, 1, 1, 1)",
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
}
