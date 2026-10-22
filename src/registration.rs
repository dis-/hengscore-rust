//! Offline registration prototype. Only explicitly marked disposable stores.
use crate::english_names::{self, CLASSES, ELEMENTS, PERSONALITIES, RACES, REALMS};
use crate::submission::{Submission, parse_submission};
use flate2::{Compression, write::GzEncoder};
use regex::Regex;
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::LazyLock,
    time::Duration,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug)]
pub enum RegistrationError {
    InvalidReference,
    IdempotencyConflict,
}
impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidReference => "unknown or ambiguous reference name",
            Self::IdempotencyConflict => "idempotency token reused with different input",
        })
    }
}
impl Error for RegistrationError {}
const MARKER: &str = "hengscore-registration-disposable-v1\n";
const JOURNAL: &str = "CREATE TABLE IF NOT EXISTS registration_poc_v1 (
 id INTEGER PRIMARY KEY, token TEXT NOT NULL UNIQUE, content_type TEXT NOT NULL,
 raw BLOB NOT NULL, dump BLOB NOT NULL, screen BLOB, done INTEGER NOT NULL DEFAULT 0 CHECK(done IN (0,1)),
 received TEXT NOT NULL DEFAULT (datetime('now','localtime')));";

pub struct Store {
    root: PathBuf,
    conn: Connection,
    // OS lock is held until Store drops, including commit/publication/recovery.
    _lock: File,
}

#[derive(Default)]
pub struct Fault(pub Option<String>);
impl Fault {
    pub fn hit(&self, point: &str) -> Result<()> {
        match self.0.as_deref() {
            Some(value) if value == format!("error:{point}") => Err("injected failure".into()),
            Some(value) if value == format!("crash:{point}") => std::process::exit(78),
            _ => Ok(()),
        }
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(value) = &self.0 {
            let (kind, point) = value.split_once(':').ok_or("invalid fault")?;
            if !["error", "crash"].contains(&kind)
                || ![
                    "before-prepare-commit",
                    "after-prepare",
                    "during-temp",
                    "after-link",
                    "after-dump",
                    "after-screen",
                    "before-commit",
                    "after-commit",
                ]
                .contains(&point)
            {
                return Err("invalid fault".into());
            }
        }
        Ok(())
    }
}

fn regular(path: &Path, directory: bool) -> Result<()> {
    let ty = fs::symlink_metadata(path)?.file_type();
    if (directory && !ty.is_dir()) || (!directory && !ty.is_file()) {
        return Err("unexpected file type or symlink".into());
    }
    Ok(())
}

fn compressed(lines: &[String]) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    // Legacy DumpFile::save writes a newline for EVERY split line, including
    // the empty final line. Preserve those exact uncompressed bytes.
    for line in lines {
        encoder.write_all(line.as_bytes())?;
        encoder.write_all(b"\n")?;
    }
    Ok(encoder.finish()?)
}

fn lookup(conn: &Connection, table: &str, id: &str, name: &str, value: &str) -> Result<i64> {
    // Identifiers are fixed call-site literals. Submitted values are bound.
    let mut stmt = conn.prepare(&format!("SELECT {id} FROM {table} WHERE {name}=?1"))?;
    let ids = stmt
        .query_map([value], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if ids.len() != 1 {
        return Err(RegistrationError::InvalidReference.into());
    }
    Ok(ids[0])
}

fn reference_name<'a>(
    ascii: bool,
    table: &'static [(&'static str, &'static str)],
    value: &'a str,
) -> Result<&'a str> {
    if !ascii {
        return Ok(value);
    }
    english_names::translate(table, value).ok_or_else(|| RegistrationError::InvalidReference.into())
}

fn references(conn: &Connection, sub: &Submission) -> Result<(i64, i64, i64, Vec<i64>)> {
    let info = &sub.info;
    let race_name = reference_name(sub.ascii, RACES, &info["race"])?;
    let class_name = reference_name(sub.ascii, CLASSES, &info["class"])?;
    let race = lookup(conn, "races", "race_id", "race_name", race_name)?;
    let class = lookup(conn, "classes", "class_id", "class_name", class_name)?;
    let personality = lookup(
        conn,
        "personalities",
        "personality_id",
        "personality_name",
        reference_name(sub.ascii, PERSONALITIES, &info["seikaku"])?,
    )?;
    // Legacy array_filter + array_values compacts out '魔法なし'.
    let mut realms = Vec::new();
    for (slot, key) in ["realm1", "realm2"].into_iter().enumerate() {
        let value = &info[key];
        let name = if sub.ascii && value == "none" {
            "魔法なし"
        } else {
            reference_name(
                sub.ascii,
                if slot == 0 && class_name == "元素使い" {
                    ELEMENTS
                } else {
                    REALMS
                },
                value,
            )?
        };
        if name != "魔法なし" {
            realms.push(lookup(conn, "realms", "realm_id", "realm_name", name)?);
        }
    }
    Ok((race, class, personality, realms))
}

fn dead_place(lines: &[String]) -> Option<String> {
    static DETAIL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^…あなたは、?(.+)。").expect("literal regex"));
    static PLACE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(.*(?:階|荒野|地上|街|辺境の地|モリバント|アングウィル|テルモラ|ズル|クエスト「.+」|クエスト)\s*)で.+$").expect("literal regex")
    });
    let text: String = lines
        .iter()
        .skip(30)
        .take(3)
        .map(|line| line.trim_matches([' ', '\t', '\n', '\r', '\0', '\u{b}']))
        .collect();
    let detail = DETAIL.captures(&text)?;
    Some(PLACE.captures(&detail[1])?[1].to_owned())
}

impl Store {
    /// Preserve legacy unkeyed semantics: each request is a separate submission.
    /// This random internal receipt is not a caller-supplied retry identity.
    pub fn register_unkeyed(
        &mut self,
        content_type: &str,
        raw: &[u8],
        fault: &Fault,
    ) -> Result<i64> {
        let token: String =
            self.conn
                .query_row("SELECT lower(hex(randomblob(32)))", [], |row| row.get(0))?;
        self.register(&token, content_type, raw, fault)
    }

    pub fn open(root: &Path) -> Result<Self> {
        Self::open_internal(root, true, None)
    }

    pub fn open_nonblocking(root: &Path) -> Result<Self> {
        Self::open_internal(root, false, None)
    }

    /// Service stores have an independently provisioned marker and identity.
    /// Neither opening a store nor starting a service creates this marker.
    pub fn open_managed_nonblocking(root: &Path, store_id: &str) -> Result<Self> {
        if store_id.len() != 32
            || !store_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid managed store identity".into());
        }
        Self::open_internal(root, false, Some(store_id))
    }

    fn open_internal(root: &Path, blocking: bool, store_id: Option<&str>) -> Result<Self> {
        regular(root, true)?;
        let (marker, expected) = match store_id {
            Some(id) => (".hengscore-store", format!("hengscore-store-v1\n{id}\n")),
            None => (".registration-poc", MARKER.to_owned()),
        };
        regular(&root.join(marker), false)?;
        if fs::read_to_string(root.join(marker))? != expected {
            return Err("store marker or identity does not match".into());
        }
        for dir in ["db", "dumps", "screens", ".staging"] {
            regular(&root.join(dir), true)?;
        }
        let lock_path = root.join(".registration.lock");
        if lock_path.try_exists()? {
            regular(&lock_path, false)?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)?;
        if blocking {
            lock.lock()?;
        } else {
            lock.try_lock()?;
        }
        let db = root.join("db/score.db");
        regular(&db, false)?;
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        conn.busy_timeout(Duration::from_secs(2))?;
        conn.execute_batch(
            "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )?;
        conn.execute_batch(JOURNAL)?;
        Ok(Self {
            root: root.to_owned(),
            conn,
            _lock: lock,
        })
    }

    fn publish(&self, id: i64, kind: &str, bytes: &[u8], fault: &Fault) -> Result<()> {
        let (directory, extension) = match kind {
            "dump" => ("dumps", "txt"),
            "screen" => ("screens", "html.bad"),
            _ => return Err("invalid kind".into()),
        };
        let parent = self
            .root
            .join(directory)
            .join((id / 1000 * 1000).to_string());
        if !parent.try_exists()? {
            fs::create_dir(&parent)?;
            File::open(self.root.join(directory))?.sync_all()?;
        }
        regular(&parent, true)?;
        let target = parent.join(format!("{id}.{extension}.gz"));
        let temporary = self.root.join(".staging").join(format!("{id}-{kind}.tmp"));
        if fs::symlink_metadata(&target).is_ok() {
            regular(&target, false)?;
            let mut existing = Vec::new();
            let file = File::open(&target)?;
            (&file)
                .take(bytes.len() as u64 + 1)
                .read_to_end(&mut existing)?;
            if existing != bytes {
                return Err("existing file conflicts; preserved for inspection".into());
            }
            file.sync_all()?;
            File::open(parent)?.sync_all()?;
            if fs::symlink_metadata(&temporary).is_ok() {
                regular(&temporary, false)?;
                fs::remove_file(&temporary)?;
                File::open(self.root.join(".staging"))?.sync_all()?;
            }
            return Ok(());
        }
        if fs::symlink_metadata(&temporary).is_ok() {
            regular(&temporary, false)?;
            fs::remove_file(&temporary)?; // Only this journal record's private staging file.
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes[..bytes.len() / 2])?;
        fault.hit("during-temp")?;
        file.write_all(&bytes[bytes.len() / 2..])?;
        file.sync_all()?;
        // Atomic no-clobber publication on the same filesystem.
        fs::hard_link(&temporary, &target)?;
        fault.hit("after-link")?;
        File::open(&parent)?.sync_all()?;
        fs::remove_file(temporary)?;
        File::open(self.root.join(".staging"))?.sync_all()?;
        Ok(())
    }

    pub fn register(
        &mut self,
        token: &str,
        content_type: &str,
        raw: &[u8],
        fault: &Fault,
    ) -> Result<i64> {
        if token.is_empty()
            || token.len() > 64
            || !token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err("invalid idempotency token".into());
        }
        let sub = parse_submission(content_type, raw)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        references(&tx, &sub)?;
        let existing = tx.query_row(
            "SELECT id, content_type, raw FROM registration_poc_v1 WHERE token=?1",
            [token],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            },
        );
        let id = match existing {
            Ok((id, header, body)) => {
                if header != content_type || body != raw {
                    return Err(RegistrationError::IdempotencyConflict.into());
                }
                id
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let max: i64 = tx.query_row("SELECT max(value) FROM (SELECT coalesce(max(score_id),0) AS value FROM scores UNION ALL SELECT coalesce(max(id),0) FROM registration_poc_v1)", [], |r| r.get(0))?;
                let id = max.checked_add(1).ok_or("score ID exhausted")?;
                let dump = compressed(&sub.dump_lines)?;
                let screen = sub.screen_lines.as_deref().map(compressed).transpose()?;
                tx.execute("INSERT INTO registration_poc_v1(id,token,content_type,raw,dump,screen) VALUES(?1,?2,?3,?4,?5,?6)",
                           params![id, token, content_type, raw, dump, screen])?;
                id
            }
            Err(error) => return Err(error.into()),
        };
        fault.hit("before-prepare-commit")?;
        tx.commit()?;
        fault.hit("after-prepare")?;
        self.finish(id, fault)?;
        Ok(id)
    }

    fn finish(&mut self, id: i64, fault: &Fault) -> Result<()> {
        let (header, raw, dump, screen, done, date) = self.conn.query_row(
            "SELECT content_type,raw,dump,screen,done,received FROM registration_poc_v1 WHERE id=?1", [id], |r|
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, Option<Vec<u8>>>(3)?, r.get::<_, bool>(4)?, r.get::<_, String>(5)?)))?;
        self.publish(id, "dump", &dump, fault)?;
        fault.hit("after-dump")?;
        if let Some(screen) = screen {
            self.publish(id, "screen", &screen, fault)?;
        }
        fault.hit("after-screen")?;
        if done {
            let present: bool = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM scores WHERE score_id=?1)",
                [id],
                |r| r.get(0),
            )?;
            if !present {
                return Err("completed journal is missing its score".into());
            }
            return Ok(());
        }
        let sub = parse_submission(&header, &raw)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (race, class, personality, realms) = references(&tx, &sub)?;
        let info = &sub.info;
        let number = |key: &str| -> Result<i64> { Ok(info[key].parse()?) };
        let winner = matches!(info["killer"].as_str(), "ripe" | "Seppuku");
        tx.execute("INSERT INTO scores(score_id,version,score,name,race_id,class_id,personality_id,sex,level,depth,maxlv,maxdp,au,turns,winner,killer,date,dead_place)
                    VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![id, info["version"], number("score")?, info["name"], race, class, personality, number("sex")?, number("level")?, number("depth")?, number("maxlv")?, number("maxdp")?, number("au")?, number("turns")?, winner, info["killer"], date, dead_place(&sub.dump_lines)])?;
        for (seq, realm) in realms.iter().enumerate() {
            tx.execute(
                "INSERT INTO score_realms(score_id,realm_id,realm_seq) VALUES(?1,?2,?3)",
                params![id, realm, (seq + 1) as i64],
            )?;
        }
        for cache in ["killers_cache", "statistics_cache"] {
            tx.execute(
                "INSERT OR REPLACE INTO cache_status_table(table_name,cached) VALUES(?1,0)",
                [cache],
            )?;
        }
        tx.execute("UPDATE registration_poc_v1 SET done=1 WHERE id=?1", [id])?;
        fault.hit("before-commit")?;
        tx.commit()?;
        fault.hit("after-commit")?;
        Ok(())
    }

    pub fn recover(&mut self, fault: &Fault) -> Result<usize> {
        let ids = self
            .conn
            .prepare("SELECT id FROM registration_poc_v1 WHERE done=0 ORDER BY id")?
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for id in &ids {
            self.finish(*id, fault)?;
        }
        Ok(ids.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn death_place_legacy_lines() {
        let mut lines = vec![String::new(); 34];
        lines[30] = " …あなたは、鉄獄の地下100階で".into();
        lines[31] = " 混沌のサーペントに倒された。 ".into();
        assert_eq!(dead_place(&lines).as_deref(), Some("鉄獄の地下100階"));
        lines[30] = "…あなたはモリバントで".into();
        assert_eq!(dead_place(&lines).as_deref(), Some("モリバント"));
        lines[30] = "…あなたは勝利して引退した。".into();
        assert_eq!(dead_place(&lines), None);
    }
}
