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

// Colors are variables so the dark palette can replace them in one place.
macro_rules! dark_palette {
    () => {
        "color-scheme:dark;--bg:#11171c;--text:#dbe3e9;--head:#0a1217;--head-sub:#9db0bd;--link:#6cc6e3;--accent:#2b8fb0;--surface:#182128;--chip-border:#2c3a45;--card-border:#26323b;--muted:#98abb8;--line:#25313a;--th-bg:#1f2b34;--th-text:#b8c8d3;--hover:#1c2730;--stripe:#1b252d;--stripe-hover:#253540;--input-border:#3d4e5a;--input-bg:#10171c"
    };
}
const STYLE: &str = concat!(
    ":root{color-scheme:light;--bg:#f5f6f8;--text:#17222e;--head:#172c3a;--head-sub:#c1d1db;--link:#086b89;--accent:#086b89;--surface:white;--chip-border:#d7e0e6;--card-border:#dfe5e9;--muted:#586b79;--line:#e8edf0;--th-bg:#edf3f6;--th-text:#425868;--hover:#f7fafb;--stripe:#eef4f7;--stripe-hover:#e1edf3;--input-border:#b9c9d2;--input-bg:white}",
    "@media(prefers-color-scheme:dark){:root:not([data-theme=light]){",
    dark_palette!(),
    "}}:root[data-theme=dark]{",
    dark_palette!(),
    "}",
    "body{margin:0;background:var(--bg);color:var(--text);font:15px/1.65 system-ui,sans-serif}header{background:var(--head);color:white;padding:24px max(24px,calc((100% - 1280px)/2))}header h1{margin:0;font-size:26px}header p{color:var(--head-sub);margin:4px 0}.theme{color:var(--head-sub);font-size:13px}.theme a{color:inherit;margin-left:10px}.theme a[aria-current]{color:white;font-weight:600;text-decoration:underline}main{max-width:1280px;margin:28px auto;padding:0 24px}a{color:var(--link);text-decoration:none}a:hover{text-decoration:underline}.tabs,.pages,.links{display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin:18px 0}.tabs a,.pages a,.links a{background:var(--surface);border:1px solid var(--chip-border);border-radius:6px;padding:8px 16px}.tabs .active{background:var(--accent);color:white;border-color:var(--accent)}.muted{color:var(--muted)}.card{background:var(--surface);border:1px solid var(--card-border);border-radius:8px;overflow:auto}table{border-collapse:collapse;width:100%;white-space:nowrap}th,td{padding:11px 14px;border-bottom:1px solid var(--line);text-align:left}th{background:var(--th-bg);color:var(--th-text);font-weight:600}tr:hover td{background:var(--hover)}.num{text-align:right;font-variant-numeric:tabular-nums}dl{display:grid;grid-template-columns:140px 1fr;margin:0;padding:16px 24px}dt,dd{padding:9px 0;margin:0;border-bottom:1px solid var(--line)}dt{color:var(--muted)}dd{overflow-wrap:anywhere}pre{margin:0;padding:24px;font:14px/1.5 ui-monospace,monospace;white-space:pre;overflow:auto}.ranking tbody.score tr:first-child td:not(.rank){border-bottom:0}.ranking .rank{text-align:center;vertical-align:middle;font-variant-numeric:tabular-nums}.ranking tbody.score:nth-of-type(even) td{background:var(--stripe)}.ranking tbody.score:hover td{background:var(--stripe-hover)}.files{display:inline-flex;margin:0 0 0 12px}.files a{padding:2px 8px}input,select,button{font:inherit;padding:6px;border:1px solid var(--input-border);border-radius:4px}input,select{background:var(--input-bg);color:var(--text)}button{background:var(--accent);color:white;cursor:pointer}label{font-size:14px}h2{margin:0;font-size:23px}@media(max-width:600px){main{padding:0 12px}header{padding:20px 16px}dl{grid-template-columns:100px 1fr;padding:12px}.card{border-radius:5px}}"
);

const SHELL_START: &str = "<!doctype html><html lang=\"ja\">";
const THEME_SLOT: &str = "<!--theme-->";
const THEME_COOKIE: &str = "hengscore_theme";

/// Color theme chosen with the header links. `Auto` follows the OS setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Auto,
    Light,
    Dark,
}

impl Theme {
    fn parse(mode: &str) -> Option<Self> {
        match mode {
            "auto" => Some(Self::Auto),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// Reads the theme from a Cookie header; anything unknown means `Auto`.
    pub fn from_cookie(header: Option<&str>) -> Self {
        header
            .into_iter()
            .flat_map(|h| h.split(';'))
            .filter_map(|pair| pair.trim().strip_prefix(THEME_COOKIE)?.strip_prefix('='))
            .find_map(Self::parse)
            .unwrap_or(Self::Auto)
    }

    /// Returns the Set-Cookie value that stores this theme; `Auto` removes the cookie.
    pub fn set_cookie(self) -> String {
        match self {
            Self::Auto => {
                format!("{THEME_COOKIE}=; Path=/; Max-Age=0; SameSite=Lax; Secure; HttpOnly")
            }
            theme => format!(
                "{THEME_COOKIE}={}; Path=/; Max-Age=31536000; SameSite=Lax; Secure; HttpOnly",
                theme.name()
            ),
        }
    }
}

/// Parses `/theme?mode=..&back=..` into the chosen theme and the page to return to.
///
/// The return target must be a path on this site. Anything else falls back to
/// the top page, so the link cannot redirect to another host.
pub fn theme_switch(query: Option<&str>) -> Result<(Theme, String)> {
    let input = Query::parse(query)?;
    input.allow(&["mode", "back"])?;
    let theme = Theme::parse(input.get("mode", "")).ok_or(PageError::BadRequest)?;
    let back = input.get("back", "/");
    let local = back.starts_with('/')
        && !back.starts_with("//")
        && back.bytes().all(|b| b.is_ascii_graphic() && b != b'\\');
    Ok((theme, if local { back.into() } else { "/".into() }))
}

/// Marks the page with the chosen theme and fills in the header links that switch it.
///
/// Only pages built by `shell` are changed; other responses pass through as they are.
pub fn apply_theme(html: String, theme: Theme, here: &str) -> String {
    let Some(rest) = html.strip_prefix(SHELL_START) else {
        return html;
    };
    let mut links = String::from("<nav class=\"theme\" aria-label=\"表示の色\">表示:");
    for (choice, label) in [
        (Theme::Auto, "自動"),
        (Theme::Light, "ライト"),
        (Theme::Dark, "ダーク"),
    ] {
        write!(
            links,
            "<a href=\"/theme?mode={}&amp;back={}\"{}>{label}</a>",
            choice.name(),
            web_features::encode(here),
            if choice == theme {
                " aria-current=\"true\""
            } else {
                ""
            }
        )
        .expect("string write");
    }
    links.push_str("</nav>");
    let attribute = match theme {
        Theme::Auto => String::new(),
        theme => format!(" data-theme=\"{}\"", theme.name()),
    };
    format!(
        "<!doctype html><html lang=\"ja\"{attribute}>{}",
        rest.replacen(THEME_SLOT, &links, 1)
    )
}

pub(crate) fn shell(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"ja\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} · Hengband Score</title><style>{STYLE}</style></head><body><header><h1>Hengband Score</h1><p>冒険の記録・ランキング</p><!--theme--></header><main><nav class=\"links\"><a href=\"/\">スコア一覧</a><a href=\"/custom_search.php\">詳細検索</a><a href=\"/popularity_ranking.php\">人気ランキング</a><a href=\"/killer_ranking.php\">死因ランキング</a><a href=\"/feed/newcome-atom.xml\">新着フィード</a></nav>{body}</main></body></html>",
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
    body.push_str("</nav><div class=\"card\"><table class=\"ranking\"><thead><tr><th>順位</th><th>スコア</th><th>名前</th><th>Lv</th><th>種族</th><th>職業</th><th>性格</th><th>性別</th><th>バージョン</th><th>登録日時</th></tr></thead>");
    for (index, (id, _)) in rows.iter().enumerate() {
        let s = score(conn, *id)?;
        let realms = web_features::realms(conn, s.id)?;
        body.push_str(&ranking_rows(
            offset + index as i64 + 1,
            &s,
            &realms,
            &web_features::file_links(root, s.id),
        ));
    }
    if rows.is_empty() {
        body.push_str(
            "<tbody><tr><td colspan=\"10\">該当するスコアはありません。</td></tr></tbody>",
        );
    }
    body.push_str("</table></div>");
    body.push_str(&web_features::pagination(&input, offset, total, PAGE_SIZE));
    body.push_str(&web_features::search_form(conn, &input)?);
    Ok(shell("ランキング", &body))
}

/// Renders one score of the ranking as two rows in its own tbody.
///
/// The rank cell spans both rows and the background alternates per tbody,
/// so readers see which rows belong together. The second row carries the
/// death and the file links without a header, as before.
fn ranking_rows(rank: i64, s: &Score, realms: &str, files: &str) -> String {
    let class = if realms.is_empty() {
        escape(&s.class)
    } else {
        format!("{} ({})", escape(&s.class), escape(realms))
    };
    format!(
        "<tbody class=\"score\"><tr data-score-id=\"{}\"><td class=\"rank\" rowspan=\"2\">{rank}</td><td class=\"num\">{}</td><td><a href=\"/scores/{}\">{}</a></td><td class=\"num\">{}</td><td>{}</td><td>{class}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr><tr><td colspan=\"9\">{} {files}</td></tr></tbody>",
        s.id,
        s.score,
        s.id,
        escape(&s.name),
        s.level,
        escape(&s.race),
        escape(&s.personality),
        sex_label(s.sex),
        escape(&s.version),
        escape(&s.date),
        escape(&s.death),
    )
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
    fn sample_score() -> Score {
        Score {
            id: 7,
            score: 1234,
            name: "<勇者>".into(),
            race: "人間".into(),
            class: "メイジ".into(),
            personality: "ふつう".into(),
            level: 30,
            date: "2026-11-08 12:00:00".into(),
            version: "変愚蛮怒 3.0".into(),
            depth: 20,
            sex: 1,
            death: "オークに殺された".into(),
        }
    }
    #[test]
    fn ranking_rows_group_one_score() {
        let rows = ranking_rows(3, &sample_score(), "生命,自然", "<nav></nav>");
        assert!(rows.starts_with("<tbody class=\"score\"><tr data-score-id=\"7\"><td class=\"rank\" rowspan=\"2\">3</td><td class=\"num\">1234</td>"));
        assert!(rows.contains("<td>メイジ (生命,自然)</td>"));
        assert!(
            rows.contains("<tr><td colspan=\"9\">オークに殺された <nav></nav></td></tr></tbody>")
        );
        assert!(rows.contains("&lt;勇者&gt;"));
        assert_eq!(rows.matches("<tr").count(), 2);
    }
    #[test]
    fn ranking_rows_without_realm_show_class_only() {
        let rows = ranking_rows(1, &sample_score(), "", "");
        assert!(rows.contains("<td>メイジ</td>"));
        assert!(!rows.contains("()"));
    }
    #[test]
    fn theme_cookie_falls_back_to_auto() {
        assert_eq!(
            Theme::from_cookie(Some("a=1; hengscore_theme=dark")),
            Theme::Dark
        );
        assert_eq!(
            Theme::from_cookie(Some("hengscore_theme=light")),
            Theme::Light
        );
        for header in [
            None,
            Some(""),
            Some("hengscore_theme=blue"),
            Some("hengscore_themex=dark"),
        ] {
            assert_eq!(Theme::from_cookie(header), Theme::Auto);
        }
        assert!(
            Theme::Dark
                .set_cookie()
                .starts_with("hengscore_theme=dark; Path=/; Max-Age=31536000;")
        );
        assert!(
            Theme::Auto
                .set_cookie()
                .starts_with("hengscore_theme=; Path=/; Max-Age=0;")
        );
    }
    #[test]
    fn theme_switch_returns_only_to_local_paths() {
        let back = |query: &str| theme_switch(Some(query)).ok().map(|(_, back)| back);
        assert_eq!(
            back("mode=dark&back=%2F%3Fsort%3Dnewcome"),
            Some("/?sort=newcome".into())
        );
        assert_eq!(back("mode=light"), Some("/".into()));
        for target in [
            "%2F%2Fevil.example",
            "https%3A%2F%2Fevil.example",
            "%2F%5Cevil.example",
            "%2Fa%20b",
            "%2F%E3%81%82",
        ] {
            assert_eq!(
                back(&format!("mode=auto&back={target}")),
                Some("/".into()),
                "{target}"
            );
        }
        assert!(matches!(
            theme_switch(Some("mode=dark")),
            Ok((Theme::Dark, _))
        ));
        for query in ["mode=blue", "back=%2F", "mode=dark&extra=1"] {
            assert!(theme_switch(Some(query)).is_err(), "{query}");
        }
    }
    #[test]
    fn theme_marks_shell_pages_only() {
        let page = shell("t", "<p>body</p>");
        assert!(page.starts_with(SHELL_START) && page.contains(THEME_SLOT));
        let dark = apply_theme(page.clone(), Theme::Dark, "/?sort=newcome");
        assert!(dark.starts_with("<!doctype html><html lang=\"ja\" data-theme=\"dark\"><head>"));
        assert!(dark.contains("<a href=\"/theme?mode=dark&amp;back=%2F%3Fsort%3Dnewcome\" aria-current=\"true\">ダーク</a>"));
        assert!(
            dark.contains("<a href=\"/theme?mode=auto&amp;back=%2F%3Fsort%3Dnewcome\">自動</a>")
        );
        assert!(!dark.contains(THEME_SLOT));
        let auto = apply_theme(page, Theme::Auto, "/");
        assert!(auto.starts_with("<!doctype html><html lang=\"ja\"><head>"));
        assert!(auto.contains("aria-current=\"true\">自動</a>"));
        let feed = "<?xml version=\"1.0\"?><feed/>".to_owned();
        assert_eq!(apply_theme(feed.clone(), Theme::Dark, "/"), feed);
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
