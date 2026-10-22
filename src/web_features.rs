use crate::web::{PageError, Result, escape, shell};
use rusqlite::Connection;
use std::{collections::BTreeMap, fmt::Write as _, path::Path};

#[derive(Default)]
pub(crate) struct Query(pub BTreeMap<String, String>);
impl Query {
    pub fn parse(query: Option<&str>) -> Result<Self> {
        let mut result = Self::default();
        if let Some(query) = query.filter(|q| !q.is_empty()) {
            if query.len() > 16384 {
                return Err(PageError::BadRequest);
            }
            for pair in query.split('&') {
                let (k, v) = pair.split_once('=').ok_or(PageError::BadRequest)?;
                let (k, v) = (decode(k)?, decode(v)?);
                if k.len() > 32
                    || v.len() > 4096
                    || result.0.len() >= 15
                    || result.0.insert(k, v).is_some()
                {
                    return Err(PageError::BadRequest);
                }
            }
        }
        Ok(result)
    }
    pub fn allow(&self, keys: &[&str]) -> Result<()> {
        if self.0.keys().all(|k| keys.contains(&k.as_str())) {
            Ok(())
        } else {
            Err(PageError::BadRequest)
        }
    }
    pub fn get<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.0.get(key).map(String::as_str).unwrap_or(default)
    }
    pub fn link(&self, path: &str, changes: &[(&str, String)]) -> String {
        let mut values = self.0.clone();
        for (key, value) in changes {
            values.insert((*key).into(), value.clone());
        }
        let pairs: Vec<_> = values
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect();
        format!("{path}?{}", pairs.join("&"))
    }
}
fn decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return Err(PageError::BadRequest);
                }
                let hex =
                    std::str::from_utf8(&bytes[i + 1..i + 3]).map_err(|_| PageError::BadRequest)?;
                if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(PageError::BadRequest);
                }
                out.push(u8::from_str_radix(hex, 16).map_err(|_| PageError::BadRequest)?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    let text = String::from_utf8(out).map_err(|_| PageError::BadRequest)?;
    if text.chars().any(char::is_control) {
        return Err(PageError::BadRequest);
    }
    Ok(text)
}
fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            write!(out, "%{b:02X}").expect("string write");
        }
    }
    out
}
fn db<T>(value: rusqlite::Result<T>) -> Result<T> {
    value.map_err(|_| PageError::Unavailable)
}

pub(crate) fn realms(conn: &Connection, id: i64) -> Result<String> {
    let mut stmt=db(conn.prepare("SELECT realm_name FROM score_realms JOIN realms USING(realm_id) WHERE score_id=? ORDER BY realm_seq"))?;
    let rows = db(stmt.query_map([id], |r| r.get::<_, String>(0)))?;
    Ok(db(rows.collect::<rusqlite::Result<Vec<_>>>())?.join(" / "))
}
pub(crate) fn file_links(root: &Path, id: i64) -> String {
    let mut body = String::from("<nav class=\"links files\" aria-label=\"保存ファイル\">");
    for (dir, suffix, kind, label) in [
        ("dumps", "txt.gz", "dump", "キャラクターダンプ"),
        ("screens", "html.gz", "screen", "スクリーンショット"),
    ] {
        let base = root.join(dir).join((id / 1000 * 1000).to_string());
        let exists = base.join(format!("{id}.{suffix}")).is_file()
            || (kind == "screen" && base.join(format!("{id}.html.bad.gz")).is_file());
        if exists {
            write!(body, "<a href=\"/scores/{id}/{kind}\">{label}</a>").expect("string write");
        }
    }
    body.push_str("</nav>");
    body
}
pub(crate) fn pagination(q: &Query, offset: i64, total: i64, size: i64) -> String {
    let page = offset / size;
    let last = total.saturating_sub(1) / size;
    let mut body = String::from("<nav class=\"pages\" aria-label=\"ページ移動\">");
    let mut pages = vec![(0, "最初".to_string())];
    if page > 0 {
        pages.push((page - 1, "← 前へ".into()));
    }
    let first = (page - 4).max(0).min((last - 9).max(0));
    for n in first..=(first + 9).min(last) {
        pages.push((n, (n + 1).to_string()));
    }
    if page < last {
        pages.push((page + 1, "次へ →".into()));
    }
    pages.push((last, "最後".into()));
    for (n, label) in pages {
        if n == page {
            write!(body, "<span aria-current=\"page\">{label}</span>").expect("string write");
        } else {
            write!(
                body,
                "<a href=\"{}\">{label}</a>",
                escape(&q.link("/", &[("start", (n * size).to_string())]))
            )
            .expect("string write");
        }
    }
    body.push_str("</nav>");
    body
}

pub(crate) fn search_form(conn: &Connection, q: &Query) -> Result<String> {
    q.allow(&[
        "sort",
        "start",
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
    ])?;
    let mut body = String::from(
        "<section class=\"card\" style=\"padding:20px;margin-top:24px\"><h2>詳細検索</h2><form action=\"/\" method=\"get\"><div style=\"display:flex;gap:16px;flex-wrap:wrap\">",
    );
    for (key, label) in [
        ("name", "名前"),
        ("killer", "死因（部分一致）"),
        ("fd", "開始日"),
        ("td", "終了日"),
        ("last_days", "直近の日数"),
    ] {
        let kind = if ["fd", "td"].contains(&key) {
            "date"
        } else if key == "last_days" {
            "number"
        } else {
            "text"
        };
        write!(body,"<label>{label}<br><input type=\"{kind}\" name=\"{key}\" maxlength=\"4096\" value=\"{}\"></label>",escape(q.get(key,""))).expect("string write");
    }
    for (key, label, options) in [
        (
            "name_match",
            "名前の一致方法",
            vec![("strict", "完全一致"), ("partial", "部分一致")],
        ),
        (
            "sex",
            "性別",
            vec![("", "すべて"), ("1", "男性"), ("0", "女性")],
        ),
        (
            "sort",
            "並び順",
            vec![("score", "スコア順"), ("newcome", "新着順")],
        ),
    ] {
        write!(body, "<label>{label}<br><select name=\"{key}\">").expect("string write");
        for (value, text) in &options {
            write!(
                body,
                "<option value=\"{value}\"{}>{text}</option>",
                if q.get(key, options[0].0) == *value {
                    " selected"
                } else {
                    ""
                }
            )
            .expect("string write");
        }
        body.push_str("</select></label>");
    }
    for (key, label, table, id, name) in [
        ("race_id", "種族", "races", "race_id", "race_name"),
        ("class_id", "職業", "classes", "class_id", "class_name"),
        (
            "personality_id",
            "性格",
            "personalities",
            "personality_id",
            "personality_name",
        ),
        ("realm1_id", "第一領域", "realms", "realm_id", "realm_name"),
        ("realm2_id", "第二領域", "realms", "realm_id", "realm_name"),
    ] {
        // Identifiers come exclusively from the fixed list above.
        let mut stmt = db(conn.prepare(&format!(
            "SELECT {id},{name} FROM {table} WHERE {id}>0 AND {name} NOT LIKE '不明%' ORDER BY {id}"
        )))?;
        let rows = db(stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))))?;
        write!(
            body,
            "<label>{label}<br><select name=\"{key}\"><option value=\"\">すべて</option>"
        )
        .expect("string write");
        for row in rows {
            let (id, name) = db(row)?;
            write!(
                body,
                "<option value=\"{id}\"{}>{}</option>",
                if q.get(key, "") == id.to_string() {
                    " selected"
                } else {
                    ""
                },
                escape(&name)
            )
            .expect("string write");
        }
        body.push_str("</select></label>");
    }
    body.push_str("</div><p><button type=\"submit\">検索する</button> <a href=\"/custom_search.php\">条件をクリア</a></p><p class=\"muted\">開始日・終了日は両方指定してください。死因に ripe を指定すると引退、Seppuku を指定すると切腹を検索します。部分一致では % と _ がワイルドカードになります。</p></form></section>");
    Ok(body)
}

pub(crate) fn extra_detail(conn: &Connection, id: i64) -> Result<String> {
    let values = db(conn.query_row(
        "SELECT maxlv,maxdp,au,turns,winner FROM scores WHERE score_id=?",
        [id],
        |r| {
            Ok([
                r.get::<_, i64>(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
            ])
        },
    ))?;
    let mut body = String::from("<section class=\"card\"><dl>");
    for (label, value) in ["最高レベル", "最深階層", "所持金", "ターン数", "勝利フラグ"]
        .iter()
        .zip(values)
    {
        write!(body, "<dt>{label}</dt><dd>{value}</dd>").expect("string write");
    }
    body.push_str("</dl></section>");
    Ok(body)
}

pub(crate) fn popularity(conn: &Connection, query: Option<&str>) -> Result<String> {
    let q = Query::parse(query)?;
    q.allow(&["category", "sort", "direction"])?;
    let category = q.get("category", "race");
    let sort = q.get("sort", "total");
    let direction = q.get("direction", "desc");
    let metric = match sort {
        "total" => 0,
        "male" => 1,
        "female" => 2,
        "winner" => 3,
        "average" => 4,
        "max" => 5,
        _ => return Err(PageError::BadRequest),
    };
    if !["asc", "desc"].contains(&direction) {
        return Err(PageError::BadRequest);
    }
    let (select, join, group, key) = match category {
        "race" => (
            "race_id,race_name,0,''",
            "JOIN races USING(race_id)",
            "race_id",
            "race_id",
        ),
        "class" => (
            "class_id,class_name,0,''",
            "JOIN classes USING(class_id)",
            "class_id",
            "class_id",
        ),
        "personality" => (
            "personality_id,personality_name,0,''",
            "JOIN personalities USING(personality_id)",
            "personality_id",
            "personality_id",
        ),
        "realm1" | "realm2" => (
            "realm_id,realm_name,class_id,class_name",
            "JOIN score_realms USING(score_id) JOIN realms USING(realm_id) JOIN classes USING(class_id)",
            "class_id,realm_id",
            if category == "realm1" {
                "realm1_id"
            } else {
                "realm2_id"
            },
        ),
        _ => return Err(PageError::BadRequest),
    };
    let filter = if category == "realm1" {
        "WHERE realm_seq=1"
    } else if category == "realm2" {
        "WHERE realm_seq=2"
    } else {
        ""
    };
    let sql = format!(
        "SELECT {select},count(*),sum(sex=1),sum(sex=0),sum(winner=1),CAST(avg(score) AS INTEGER),max(score) FROM scores {join} {filter} GROUP BY {group}"
    );
    let mut stmt = db(conn.prepare(&sql))?;
    let rows = db(stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            [
                r.get::<_, i64>(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
            ],
        ))
    }))?;
    let mut rows = db(rows.collect::<rusqlite::Result<Vec<_>>>())?;
    rows.sort_by(|a, b| {
        let cmp = a.4[metric].cmp(&b.4[metric]);
        let cmp = if direction == "desc" {
            cmp.reverse()
        } else {
            cmp
        };
        cmp.then_with(|| a.2.cmp(&b.2)).then_with(|| a.0.cmp(&b.0))
    });
    let mut body = String::from("<h2>人気ランキング</h2><nav class=\"tabs\">");
    for (value, label) in [
        ("race", "種族"),
        ("class", "職業"),
        ("personality", "性格"),
        ("realm1", "第一領域"),
        ("realm2", "第二領域"),
    ] {
        write!(
            body,
            "<a href=\"/popularity_ranking.php?category={value}\" class=\"{}\">{label}</a>",
            if category == value { "active" } else { "" }
        )
        .expect("string write");
    }
    body.push_str("</nav><p class=\"muted\">見出しを選ぶと並べ替えます。領域は職業ごとに集計します。</p><div class=\"card\"><table><thead><tr><th>名前</th>");
    for (value, label) in [
        ("total", "総数"),
        ("male", "男性"),
        ("female", "女性"),
        ("winner", "勝利"),
        ("average", "平均スコア"),
        ("max", "最高スコア"),
    ] {
        let next = if sort == value && direction == "desc" {
            "asc"
        } else {
            "desc"
        };
        write!(
            body,
            "<th><a href=\"{}\">{label}{}</a></th>",
            escape(&q.link(
                "/popularity_ranking.php",
                &[("sort", value.into()), ("direction", next.into())]
            )),
            if sort == value {
                if direction == "desc" { " ▼" } else { " ▲" }
            } else {
                ""
            }
        )
        .expect("string write");
    }
    body.push_str("</tr></thead><tbody>");
    for (id, name, class_id, class_name, values) in rows {
        let mut filter = Query::default();
        filter.0.insert(key.into(), id.to_string());
        if category.starts_with("realm") {
            filter.0.insert("class_id".into(), class_id.to_string());
        }
        write!(body,"<tr data-category-id=\"{id}\" data-class-id=\"{class_id}\"><td><a href=\"{}\">{}{}</a></td>",escape(&filter.link("/",&[])),escape(&class_name),escape(&if class_name.is_empty(){name}else{format!(" / {name}")})).expect("string write");
        for v in values {
            write!(body, "<td class=\"num\">{v}</td>").expect("string write");
        }
        body.push_str("</tr>");
    }
    body.push_str("</tbody></table></div>");
    Ok(shell("人気ランキング", &body))
}

pub(crate) fn killers(conn: &Connection, query: Option<&str>) -> Result<String> {
    Query::parse(query)?.allow(&[])?;
    let mut stmt=db(conn.prepare("WITH deaths AS (SELECT CASE WHEN killer LIKE '麻痺状態で%' OR killer LIKE '彫像状態で%' THEN substr(killer,6) WHEN killer='ripe' THEN '引退' WHEN killer='Seppuku' THEN '切腹' ELSE coalesce(killer,'') END AS cause, CASE WHEN killer LIKE '麻痺状態で%' OR killer LIKE '彫像状態で%' THEN 1 ELSE 0 END AS frozen FROM scores) SELECT cause,count(*),sum(frozen) FROM deaths GROUP BY cause ORDER BY count(*) DESC,cause"))?;
    let rows = db(stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
        ))
    }))?;
    let mut body = String::from(
        "<h2>死因ランキング</h2><p class=\"muted\">麻痺・彫像状態の死因をまとめて集計します。</p><div class=\"card\"><table><thead><tr><th>順位</th><th>死因</th><th>総数</th><th>通常</th><th>麻痺・彫像状態</th></tr></thead><tbody>",
    );
    for (index, row) in rows.enumerate() {
        let (cause, total, frozen) = db(row)?;
        let label = &cause;
        write!(body,"<tr><td>{}</td><td>{}</td><td class=\"num\">{total}</td><td class=\"num\">{}</td><td class=\"num\">{frozen}</td></tr>",index+1,escape(label),total-frozen).expect("string write");
    }
    body.push_str("</tbody></table></div>");
    Ok(shell("死因ランキング", &body))
}

fn xml(text: &str) -> String {
    escape(text).replace("&#39;", "&apos;")
}
pub(crate) fn feed(root: &Path, conn: &Connection) -> Result<String> {
    let mut stmt=db(conn.prepare("SELECT score_id,coalesce(name,''),score,coalesce(strftime('%Y-%m-%dT%H:%M:%S',date),'1970-01-01T00:00:00') || '+09:00' FROM scores ORDER BY score_id DESC LIMIT 30"))?;
    let rows = db(stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
        ))
    }))?;
    let rows = db(rows.collect::<rusqlite::Result<Vec<_>>>())?;
    let updated = rows
        .first()
        .map(|r| r.3.as_str())
        .unwrap_or("1970-01-01T00:00:00+09:00");
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"><id>urn:hengband:scores</id><title>Hengband 新着スコア</title><author><name>Hengband Score</name></author><updated>{updated}</updated><link rel=\"self\" href=\"/feed/newcome-atom.xml\"/>"
    );
    for (id, name, score, date) in rows {
        let summary=db(conn.query_row("SELECT personality_name,race_name,class_name,CASE WHEN killer='ripe' THEN '勝利の後引退' WHEN killer='Seppuku' THEN '勝利の後切腹' ELSE coalesce(dead_place || 'で','') || coalesce(killer,'') || 'に殺された' END,depth FROM scores JOIN races USING(race_id) JOIN classes USING(class_id) JOIN personalities USING(personality_id) WHERE score_id=?",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?))))?;
        let dump = crate::web::file_text(root, id, false).unwrap_or_default();
        let death_lines = dump
            .lines()
            .skip(30)
            .take(3)
            .map(str::trim)
            .collect::<String>();
        let detail = death_lines
            .strip_prefix("…あなたは")
            .and_then(|s| {
                s.trim_start_matches('、')
                    .split_once('。')
                    .map(|p| p.0.to_owned())
            })
            .unwrap_or_else(|| format!("{} @{}F", summary.3, summary.4));
        let title = format!(
            "{}{name} Score:{score} {} {} {detail}",
            summary.0, summary.1, summary.2
        );
        write!(out,"<entry><id>urn:hengband:score:{id}</id><title>{}</title><updated>{date}</updated><link href=\"/scores/{id}/dump\"/><content type=\"text\">{}</content></entry>",xml(&title),xml(&last_message(&dump))).expect("string write");
    }
    out.push_str("</feed>");
    Ok(out)
}

fn last_message(dump: &str) -> String {
    let mut found = false;
    let mut out = String::new();
    for line in dump.lines() {
        let trimmed = line.trim();
        if let Some(header) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if [
                "*勝利*メッセージ",
                "死ぬ直前のメッセージ",
                "Last Messages",
                "*WINNER* message",
            ]
            .contains(&header)
            {
                found = true;
            } else if found {
                break;
            }
        }
        if found {
            if out.len() + line.len() > 65536 {
                out.push_str("\n（表示上限のため省略）");
                break;
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_query_decoding_and_roundtrip() {
        for q in [
            "name=%",
            "name=%+1",
            "name=%0x",
            "name=%FF",
            "name=%00",
            "name=a&name=b",
            "%73ort=score&sort=newcome",
        ] {
            assert!(Query::parse(Some(q)).is_err());
        }
        let q = Query::parse(Some("name=%E5%8B%87%E8%80%85+a%26b&sex=0"))
            .unwrap_or_else(|_| panic!("valid query"));
        assert_eq!(q.get("name", ""), "勇者 a&b");
        assert_eq!(q.get("sex", ""), "0");
        let link = q.link("/", &[("start", "50".into())]);
        let next = Query::parse(link.split_once('?').map(|p| p.1))
            .unwrap_or_else(|_| panic!("valid link"));
        assert_eq!(q.get("name", ""), next.get("name", ""));
    }
    #[test]
    fn decimal_id_validation() {
        assert!(crate::web::digits("1 OR 1=1").is_err());
    }

    #[test]
    fn message_section_stops_before_next_section() {
        assert_eq!(
            last_message("ignored\n[死ぬ直前のメッセージ]\n<script>x</script>\n[装備]\nsecret"),
            "[死ぬ直前のメッセージ]\n<script>x</script>\n"
        );
        assert_eq!(
            last_message("[*勝利*メッセージ]\n勝利\n[装備]"),
            "[*勝利*メッセージ]\n勝利\n"
        );
        assert!(last_message("no message").is_empty());
    }

    #[test]
    fn pagination_never_creates_negative_offsets() {
        let q = Query::parse(Some("sex=0&name=%E5%8B%87%E8%80%85"))
            .unwrap_or_else(|_| panic!("valid query"));
        for (offset, total) in [(0, 0), (0, 1), (0, 51), (50, 51), (999999, 100)] {
            let page = pagination(&q, offset, total, 50);
            assert!(!page.contains("start=-"));
            assert!(!page.contains("start=%2D"));
            assert!(page.contains("sex=0") || total <= 50);
        }
    }
}
