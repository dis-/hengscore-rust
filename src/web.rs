use crate::search::{Filters, PAGE_SIZE, SortMode, page_offset, search};
use crate::web_features::{self, Query};
use flate2::read::GzDecoder;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{
    fmt::Write as _,
    fs::{self, File},
    io::Read,
    path::Path,
    time::Duration,
};

pub enum PageError {
    BadRequest,
    NotFound,
    TooLarge,
    Unavailable,
}
pub(crate) type Result<T> = std::result::Result<T, PageError>;
const MAX_FILE: usize = 8 * 1024 * 1024;

fn sex_label(value: i64) -> &'static str {
    match value {
        0 => "女性",
        1 => "男性",
        _ => "不明",
    }
}

pub fn escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if c.is_control() && !['\n', '\r', '\t'].contains(&c) => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

const STYLE: &str = "body{margin:0;background:#f5f6f8;color:#17222e;font:15px/1.65 system-ui,sans-serif}header{background:#172c3a;color:white;padding:24px max(24px,calc((100% - 1280px)/2))}header h1{margin:0;font-size:26px}header p{color:#c1d1db;margin:4px 0}main{max-width:1280px;margin:28px auto;padding:0 24px}a{color:#086b89;text-decoration:none}a:hover{text-decoration:underline}.tabs,.pages,.links{display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin:18px 0}.tabs a,.pages a,.links a{background:white;border:1px solid #d7e0e6;border-radius:6px;padding:8px 16px}.tabs .active{background:#086b89;color:white;border-color:#086b89}.muted{color:#586b79}.card{background:white;border:1px solid #dfe5e9;border-radius:8px;overflow:auto}table{border-collapse:collapse;width:100%;white-space:nowrap}th,td{padding:11px 14px;border-bottom:1px solid #e8edf0;text-align:left}th{background:#edf3f6;color:#425868;font-weight:600}tr:hover td{background:#f7fafb}.num{text-align:right;font-variant-numeric:tabular-nums}dl{display:grid;grid-template-columns:140px 1fr;margin:0;padding:16px 24px}dt,dd{padding:9px 0;margin:0;border-bottom:1px solid #edf0f2}dt{color:#586b79}dd{overflow-wrap:anywhere}pre{margin:0;padding:24px;font:14px/1.5 ui-monospace,monospace;white-space:pre;overflow:auto}.files{display:inline-flex;margin:0 0 0 12px}.files a{padding:2px 8px}input,select,button{font:inherit;padding:6px;border:1px solid #b9c9d2;border-radius:4px}button{background:#086b89;color:white;cursor:pointer}label{font-size:14px}h2{margin:0;font-size:23px}@media(max-width:600px){main{padding:0 12px}header{padding:20px 16px}dl{grid-template-columns:100px 1fr;padding:12px}.card{border-radius:5px}}";

pub(crate) fn shell(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"ja\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} · Hengband Score</title><style>{STYLE}</style></head><body><header><h1>Hengband Score</h1><p>ローカル表示試験 · 読み取り専用</p></header><main><nav class=\"links\"><a href=\"/\">スコア一覧</a><a href=\"/custom_search.php\">詳細検索</a><a href=\"/popularity_ranking.php\">人気ランキング</a><a href=\"/killer_ranking.php\">死因ランキング</a><a href=\"/feed/newcome-atom.xml\">新着フィード</a></nav>{body}</main></body></html>",
        escape(title)
    )
}

pub fn error_page(error: &PageError) -> String {
    let message = match error {
        PageError::BadRequest => "指定された条件を確認してください。",
        PageError::NotFound => "スコアまたはファイルが見つかりません。",
        PageError::TooLarge => "このファイルは表示上限を超えています。",
        PageError::Unavailable => "現在、データを読み取れません。",
    };
    shell(
        "表示できません",
        &format!("<h2>表示できません</h2><p>{message}</p><p><a href=\"/\">一覧へ戻る</a></p>"),
    )
}

fn open(root: &Path) -> Result<Connection> {
    let conn =
        Connection::open_with_flags(root.join("db/score.db"), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| PageError::Unavailable)?;
    conn.busy_timeout(Duration::from_secs(2))
        .map_err(|_| PageError::Unavailable)?;
    Ok(conn)
}

struct Score {
    id: i64,
    score: i64,
    name: String,
    race: String,
    class: String,
    personality: String,
    level: i64,
    date: String,
    version: String,
    depth: i64,
    sex: i64,
    death: String,
}
fn score(conn: &Connection, id: i64) -> Result<Score> {
    conn.query_row("SELECT score_id,score,coalesce(name,''),race_name,class_name,personality_name,level,date,version,coalesce(killer,''),depth,sex,CASE WHEN killer='ripe' THEN '勝利の後引退' WHEN killer='Seppuku' THEN '勝利の後切腹' WHEN dead_place IS NULL THEN coalesce(killer,'') || 'に殺された' ELSE dead_place || 'で' || coalesce(killer,'') || 'に殺された' END FROM scores JOIN races USING(race_id) JOIN classes USING(class_id) JOIN personalities USING(personality_id) WHERE score_id=?1",[id],|r| Ok(Score{id:r.get(0)?,score:r.get(1)?,name:r.get(2)?,race:r.get(3)?,class:r.get(4)?,personality:r.get(5)?,level:r.get(6)?,date:r.get(7)?,version:r.get(8)?,depth:r.get(10)?,sex:r.get(11)?,death:r.get(12)?}))
        .optional().map_err(|_|PageError::Unavailable)?.ok_or(PageError::NotFound)
}

pub(crate) fn digits(text: &str) -> Result<i64> {
    if text.is_empty() || text.len() > 19 || !text.bytes().all(|c| c.is_ascii_digit()) {
        return Err(PageError::BadRequest);
    }
    text.parse().map_err(|_| PageError::BadRequest)
}

fn list(root: &Path, conn: &Connection, query: Option<&str>) -> Result<String> {
    let input = Query::parse(query)?;
    input.allow(&[
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
    let sort = match input.get("sort", "score") {
        "socre" => "score",
        value => value,
    };
    let start = input.get("start", "0");
    digits(start)?;
    let offset = page_offset(start).map_err(|_| PageError::BadRequest)?;
    let mode = SortMode::parse(sort).map_err(|_| PageError::BadRequest)?;
    let filters = Filters::parse(
        input
            .0
            .iter()
            .filter(|(k, _)| !["sort", "start"].contains(&k.as_str()))
            .map(|(k, v)| format!("{k}={v}"))
            .collect(),
    )
    .map_err(|_| PageError::BadRequest)?;
    let result = search(conn, mode, offset, &filters).map_err(|_| PageError::Unavailable)?;
    let rows = result.rows;
    let total = result.total;
    let mut body = format!(
        "<h2>スコアランキング</h2><p class=\"muted\">該当 {total} 件 · 1ページ50件</p><nav class=\"tabs\" aria-label=\"並び順\">"
    );
    for (key, label) in [("score", "スコア順"), ("newcome", "新着順")] {
        write!(
            body,
            "<a class=\"{}\" href=\"{}\">{label}</a>",
            if key == sort { "active" } else { "" },
            escape(&input.link("/", &[("sort", key.to_string()), ("start", "0".into())]))
        )
        .expect("string write");
    }
    body.push_str("</nav><div class=\"card\"><table><thead><tr><th>順位</th><th>スコア</th><th>名前</th><th>Lv</th><th>種族</th><th>職業</th><th>性格</th><th>性別</th><th>バージョン</th><th>登録日時</th></tr></thead><tbody>");
    for (index, (id, _)) in rows.iter().enumerate() {
        let s = score(conn, *id)?;
        write!(body,"<tr data-score-id=\"{}\"><td class=\"num\">{}</td><td class=\"num\">{}</td><td><a href=\"/scores/{}\">{}</a></td><td class=\"num\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",s.id,offset+index as i64+1,s.score,s.id,escape(&s.name),s.level,escape(&s.race),escape(&s.class),escape(&s.personality),sex_label(s.sex),escape(&s.version),escape(&s.date)).expect("string write");
        let realms = web_features::realms(conn, s.id)?;
        write!(
            body,
            "<tr><td></td><td colspan=\"9\">{} · {} {} </td></tr>",
            escape(&s.death),
            escape(&realms),
            web_features::file_links(root, s.id)
        )
        .expect("string write");
    }
    if rows.is_empty() {
        body.push_str("<tr><td colspan=\"10\">該当するスコアはありません。</td></tr>");
    }
    body.push_str("</tbody></table></div>");
    body.push_str(&web_features::pagination(&input, offset, total, PAGE_SIZE));
    body.push_str(&web_features::search_form(conn, &input)?);
    Ok(shell("ランキング", &body))
}

fn detail(root: &Path, conn: &Connection, id: i64) -> Result<String> {
    let s = score(conn, id)?;
    let mut stmt=conn.prepare("SELECT realm_name FROM score_realms JOIN realms USING(realm_id) WHERE score_id=? ORDER BY realm_seq").map_err(|_|PageError::Unavailable)?;
    let realms = stmt
        .query_map([id], |r| r.get::<_, String>(0))
        .map_err(|_| PageError::Unavailable)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| PageError::Unavailable)?
        .join(" / ");
    let mut body = format!(
        "<p><a href=\"/\">← 一覧へ戻る</a></p><h2>{}</h2><p class=\"muted\">スコアID {id}</p><section class=\"card\"><dl>",
        escape(&s.name)
    );
    for (label, value) in [
        ("スコア", s.score.to_string()),
        ("レベル", s.level.to_string()),
        ("種族", s.race),
        ("職業", s.class),
        ("性格", s.personality),
        (
            "領域",
            if realms.is_empty() {
                "魔法なし".into()
            } else {
                realms
            },
        ),
        ("性別", sex_label(s.sex).into()),
        ("階層", s.depth.to_string()),
        ("死因 / 結果", s.death),
        ("バージョン", s.version),
        ("登録日時", s.date),
    ] {
        write!(body, "<dt>{label}</dt><dd>{}</dd>", escape(&value)).expect("string write");
    }
    body.push_str("</dl></section>");
    body.push_str(&web_features::extra_detail(conn, id)?);
    body.push_str(&web_features::file_links(root, id));
    Ok(shell("スコア詳細", &body))
}

pub(crate) fn file_text(root: &Path, id: i64, screen: bool) -> Result<String> {
    let directory = root
        .join(if screen { "screens" } else { "dumps" })
        .join((id / 1000 * 1000).to_string());
    let candidates = if screen {
        vec![
            directory.join(format!("{id}.html.gz")),
            directory.join(format!("{id}.html.bad.gz")),
        ]
    } else {
        vec![directory.join(format!("{id}.txt.gz"))]
    };
    let path = candidates
        .into_iter()
        .find(|p| fs::symlink_metadata(p).is_ok())
        .ok_or(PageError::NotFound)?;
    if !fs::symlink_metadata(&path)
        .map_err(|_| PageError::Unavailable)?
        .is_file()
    {
        return Err(PageError::Unavailable);
    }
    let mut compressed = Vec::new();
    File::open(path)
        .map_err(|_| PageError::Unavailable)?
        .take((MAX_FILE + 1) as u64)
        .read_to_end(&mut compressed)
        .map_err(|_| PageError::Unavailable)?;
    if compressed.len() > MAX_FILE {
        return Err(PageError::TooLarge);
    }
    let mut decoded = Vec::new();
    GzDecoder::new(compressed.as_slice())
        .take((MAX_FILE + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|_| PageError::Unavailable)?;
    if decoded.len() > MAX_FILE {
        return Err(PageError::TooLarge);
    }
    String::from_utf8(decoded).map_err(|_| PageError::Unavailable)
}

fn sanitized_screen(text: &str) -> String {
    use std::{
        borrow::Cow,
        collections::{HashMap, HashSet},
    };
    let mut cleaner = ammonia::Builder::default();
    cleaner
        .tags(HashSet::from(["pre", "font", "span", "br"]))
        .generic_attributes(HashSet::new())
        .tag_attributes(HashMap::from([("font", HashSet::from(["color"]))]))
        .link_rel(None)
        .clean_content_tags(HashSet::from([
            "script",
            "style",
            "iframe",
            "object",
            "template",
            "svg",
            "math",
            "textarea",
            "title",
            "noscript",
            "xmp",
            "plaintext",
        ]))
        .attribute_filter(|tag, attribute, value| {
            if tag == "font"
                && attribute == "color"
                && [4, 7].contains(&value.len())
                && value.starts_with('#')
                && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
            {
                Some(Cow::Borrowed(value))
            } else {
                None
            }
        });
    cleaner.clean(text).to_string()
}

fn screen_frame(text: &str) -> String {
    let safe = sanitized_screen(text);
    let frame = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><style>body{{margin:0;padding:12px;background:#000;color:#fff}}pre{{margin:0;font:14px/1.2 'MS Gothic','Osaka-Mono',monospace;white-space:pre}}</style></head><body>{safe}</body></html>"
    );
    format!(
        "<iframe title=\"ゲーム画面\" sandbox=\"\" referrerpolicy=\"no-referrer\" style=\"display:block;width:100%;height:80vh;min-height:540px;border:0;background:#000\" srcdoc=\"{}\"></iframe>",
        escape(&frame)
    )
}

pub fn render(root: &Path, path: &str, query: Option<&str>) -> Result<String> {
    let mut conn = open(root)?;
    // One response observes one committed SQLite snapshot, even while a post
    // publishes its score and realm rows in another connection.
    let tx = conn.transaction().map_err(|_| PageError::Unavailable)?;
    render_snapshot(root, &tx, path, query)
}

fn render_snapshot(
    root: &Path,
    conn: &Connection,
    path: &str,
    query: Option<&str>,
) -> Result<String> {
    if ["/", "/score_ranking.php"].contains(&path) {
        return list(root, conn, query);
    }
    if path == "/custom_search.php" {
        let input = Query::parse(query)?;
        return Ok(shell("詳細検索", &web_features::search_form(conn, &input)?));
    }
    if path == "/popularity_ranking.php" {
        return web_features::popularity(conn, query);
    }
    if path == "/killer_ranking.php" {
        return web_features::killers(conn, query);
    }
    if path == "/feed/newcome-atom.xml" {
        if query.is_some() {
            return Err(PageError::BadRequest);
        }
        return web_features::feed(root, conn);
    }
    if ["/show_dump.php", "/show_screen.php"].contains(&path) {
        let input = Query::parse(query)?;
        input.allow(&["score_id"])?;
        let id = digits(input.get("score_id", ""))?;
        return render_snapshot(
            root,
            conn,
            &format!(
                "/scores/{id}/{}",
                if path == "/show_dump.php" {
                    "dump"
                } else {
                    "screen"
                }
            ),
            None,
        );
    }
    if query.is_some() {
        return Err(PageError::BadRequest);
    }
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    if parts.len() < 2 || parts[0] != "scores" {
        return Err(PageError::NotFound);
    }
    let id = digits(parts[1])?;
    match parts.as_slice() {
        [_, _] => detail(root, conn, id),
        [_, _, kind] if ["dump", "screen"].contains(kind) => {
            score(conn, id)?;
            let text = file_text(root, id, *kind == "screen")?;
            let title = if *kind == "screen" {
                "スクリーンショット"
            } else {
                "キャラクターダンプ"
            };
            let content = if *kind == "screen" {
                screen_frame(&text)
            } else {
                format!("<pre>{}</pre>", escape(&text))
            };
            let body = format!(
                "<p><a href=\"/scores/{id}\">← 詳細へ戻る</a></p><h2>{title}</h2><p class=\"muted\">スコアID {id}</p><div class=\"card\">{content}</div>"
            );
            Ok(shell(title, &body))
        }
        _ => Err(PageError::NotFound),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screen_keeps_game_colors_and_blocks_active_html() {
        let raw = r##"<html><body bgcolor="#000000"><pre><font color="#00ff00" onclick="alert(1)">@&lt;</font><font color="red;url(https://bad)">X</font><span style="color:red">.</span></pre><script>alert(1)</script><img src="https://bad"><meta http-equiv="refresh" content="0;url=https://bad"><a href="https://bad">link</a><svg onload="alert(1)"></svg></body></html>"##;
        let safe = sanitized_screen(raw);
        assert!(safe.contains(r##"<font color="#00ff00">@&lt;</font>"##));
        for forbidden in [
            "script", "onclick", "onload", "style=", "url(", "https://", "<img", "<meta", "<a ",
            "<svg",
        ] {
            assert!(!safe.contains(forbidden), "{forbidden}: {safe}");
        }
        assert!(screen_frame(raw).contains("sandbox=\"\""));
    }
    #[test]
    fn sex_matches_game_enum() {
        assert_eq!(sex_label(0), "女性");
        assert_eq!(sex_label(1), "男性");
        assert_eq!(sex_label(2), "不明");
    }
    #[test]
    fn untrusted_text_is_escaped() {
        assert_eq!(
            escape("</pre><script>'\"&\0"),
            "&lt;/pre&gt;&lt;script&gt;&#39;&quot;&amp;�"
        );
    }
    #[test]
    fn numeric_paths_are_strict() {
        for value in ["", "../1", "-1", "1'", "9223372036854775808"] {
            assert!(digits(value).is_err());
        }
        assert_eq!(digits("0").ok(), Some(0));
    }
}
