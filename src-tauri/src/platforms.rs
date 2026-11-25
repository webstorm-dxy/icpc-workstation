use crate::automation::AutomationManager;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::models::{AutomationBounds, ProblemDto, SampleDto, SettingsDto, SyncProgressDto};
use crate::util::{
    html_to_text, now_iso, truncate_chars, verdict_from_atcoder, verdict_from_cf,
    verdict_from_luogu,
};
use chrono::{TimeZone, Utc};
use regex::Regex;
use scraper::{Html, Selector};
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

pub const PLATFORMS: [&str; 3] = ["codeforces", "atcoder", "luogu"];

pub fn platform_name(platform: &str) -> &'static str {
    match platform {
        "codeforces" => "Codeforces",
        "atcoder" => "AtCoder",
        "luogu" => "洛谷",
        _ => "未知平台",
    }
}

pub fn login_url(platform: &str) -> &'static str {
    match platform {
        "codeforces" => "https://codeforces.com/enter",
        "atcoder" => "https://atcoder.jp/login",
        "luogu" => "https://www.luogu.com.cn/auth/login",
        _ => "https://example.com",
    }
}

pub fn login_probe(platform: &str) -> &'static str {
    match platform {
        "codeforces" => {
            r#"(() => {
  const profile = document.querySelector('a[href^="/profile/"]');
  const logout = !!document.querySelector('a[href="/logout"]');
  const handle = profile ? profile.getAttribute('href').split('/').filter(Boolean).pop() : null;
  return JSON.stringify({ logged: logout || !!handle, handle, uid: null });
})()"#
        }
        "atcoder" => {
            r#"(() => {
  const logout = !!document.querySelector('a[href="/logout"]') || document.body.innerText.includes('ログアウト');
  let handle = null;
  for (const a of document.querySelectorAll('a[href^="/users/"]')) {
    const parts = a.getAttribute('href').split('/').filter(Boolean);
    if (parts.length === 2 && parts[1] && parts[1] !== 'settings') { handle = parts[1]; break; }
  }
  return JSON.stringify({ logged: logout || !!handle, handle, uid: null });
})()"#
        }
        "luogu" => {
            r#"(() => {
  const text = document.body ? document.body.innerText : '';
  const logged = location.pathname.indexOf('/auth/login') === -1 && !/登录\s*\/\s*注册/.test(text) && (!!document.querySelector('a[href*="/auth/logout"], .user-nav, .lg-avatar, img.avatar'));
  let uid = null;
  for (const a of document.querySelectorAll('a[href^="/user/"]')) {
    const m = a.getAttribute('href').match(/^\/user\/(\d+)/);
    if (m) { uid = Number(m[1]); break; }
  }
  return JSON.stringify({ logged, handle: null, uid });
})()"#
        }
        _ => "JSON.stringify({ logged: false, handle: null, uid: null })",
    }
}

pub fn challenge_probe() -> &'static str {
    r#"(() => {
  const text = document.body ? document.body.innerText : '';
  const challenge = /(验证码|人机验证|安全验证|访问过于频繁|Just a moment|Checking your browser|Verify you are human|Enable JavaScript and cookies)/i.test(text)
    || !!document.querySelector('iframe[src*="captcha"], .cf-turnstile, .h-captcha, .g-recaptcha, #challenge-form, #cf-challenge-running, .nc_wrapper');
  const login = !!document.querySelector('input[type="password"], form[action*="login"], form[action*="enter"]');
  return JSON.stringify({ challenge, login, url: location.href, title: document.title });
})()"#
}

pub const DEFAULT_BOUNDS: AutomationBounds = AutomationBounds {
    x: 0.0,
    y: 0.0,
    width: 720.0,
    height: 640.0,
    viewport_height: None,
};

#[derive(Debug, Clone)]
pub struct Account {
    pub platform: String,
    pub handle: Option<String>,
    pub uid: Option<String>,
    pub logged_in: bool,
    pub rating: i64,
    pub max_rating: i64,
    pub solved_count: i64,
    pub meta: Value,
    pub last_synced_at: Option<String>,
}

pub async fn load_account(db: &Db, platform: &str) -> AppResult<Option<Account>> {
    let row = sqlx::query(
        "SELECT platform, handle, uid, logged_in, rating, max_rating, solved_count, meta_json, last_synced_at
         FROM platform_accounts WHERE platform = ?1",
    )
    .bind(platform)
    .fetch_optional(&db.pool)
    .await?;
    Ok(row.map(|row| Account {
        platform: row.get("platform"),
        handle: row.get("handle"),
        uid: row.get("uid"),
        logged_in: row.get::<i64, _>("logged_in") != 0,
        rating: row.get::<Option<i64>, _>("rating").unwrap_or(0),
        max_rating: row.get::<Option<i64>, _>("max_rating").unwrap_or(0),
        solved_count: row.get::<Option<i64>, _>("solved_count").unwrap_or(0),
        meta: serde_json::from_str(row.get::<&str, _>("meta_json")).unwrap_or(json!({})),
        last_synced_at: row.get("last_synced_at"),
    }))
}

pub async fn save_account(db: &Db, account: &Account) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO platform_accounts(platform, handle, uid, logged_in, rating, max_rating, solved_count, meta_json, last_synced_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(platform) DO UPDATE SET
           handle = excluded.handle,
           uid = excluded.uid,
           logged_in = excluded.logged_in,
           rating = excluded.rating,
           max_rating = excluded.max_rating,
           solved_count = excluded.solved_count,
           meta_json = excluded.meta_json,
           last_synced_at = excluded.last_synced_at",
    )
    .bind(&account.platform)
    .bind(&account.handle)
    .bind(&account.uid)
    .bind(if account.logged_in { 1 } else { 0 })
    .bind(account.rating)
    .bind(account.max_rating)
    .bind(account.solved_count)
    .bind(account.meta.to_string())
    .bind(&account.last_synced_at)
    .execute(&db.pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ProblemSeed {
    pub id: String,
    pub platform: String,
    pub native_id: String,
    pub title: String,
    pub url: String,
    pub rating: Option<i64>,
    pub normalized_rating: Option<i64>,
    pub tags: Vec<String>,
    pub skill_tags: Vec<String>,
    pub time_limit_ms: Option<i64>,
    pub memory_limit_mb: Option<i64>,
}

pub async fn upsert_problems(db: &Db, seeds: &[ProblemSeed]) -> AppResult<()> {
    if seeds.is_empty() {
        return Ok(());
    }
    let mut tx = db.pool.begin().await?;
    for seed in seeds {
        sqlx::query(
            "INSERT INTO problems(id, platform, native_id, title, url, rating, normalized_rating, tags_json, skill_tags_json, time_limit_ms, memory_limit_mb)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
               title = excluded.title,
               url = excluded.url,
               rating = COALESCE(excluded.rating, problems.rating),
               normalized_rating = COALESCE(excluded.normalized_rating, problems.normalized_rating),
               tags_json = CASE WHEN excluded.tags_json = '[]' THEN problems.tags_json ELSE excluded.tags_json END,
               skill_tags_json = CASE WHEN excluded.skill_tags_json = '[]' THEN problems.skill_tags_json ELSE excluded.skill_tags_json END,
               time_limit_ms = COALESCE(excluded.time_limit_ms, problems.time_limit_ms),
               memory_limit_mb = COALESCE(excluded.memory_limit_mb, problems.memory_limit_mb)",
        )
        .bind(&seed.id)
        .bind(&seed.platform)
        .bind(&seed.native_id)
        .bind(&seed.title)
        .bind(&seed.url)
        .bind(seed.rating)
        .bind(seed.normalized_rating)
        .bind(serde_json::to_string(&seed.tags).unwrap_or_else(|_| "[]".into()))
        .bind(serde_json::to_string(&seed.skill_tags).unwrap_or_else(|_| "[]".into()))
        .bind(seed.time_limit_ms)
        .bind(seed.memory_limit_mb)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct SubmissionSeed {
    pub id: String,
    pub platform: String,
    pub native_id: String,
    pub problem_id: String,
    pub verdict: String,
    pub language: Option<String>,
    pub time_ms: Option<i64>,
    pub memory_kb: Option<i64>,
    pub submitted_at: String,
    pub raw: Value,
}

pub async fn upsert_submissions(db: &Db, seeds: &[SubmissionSeed]) -> AppResult<()> {
    if seeds.is_empty() {
        return Ok(());
    }
    let mut tx = db.pool.begin().await?;
    for seed in seeds {
        sqlx::query(
            "INSERT INTO submissions(id, platform, native_id, problem_id, verdict, language, time_ms, memory_kb, submitted_at, captured_via, raw_json, analysis_status)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'sync', ?10, 'pending')
             ON CONFLICT(platform, native_id) DO UPDATE SET
               verdict = excluded.verdict,
               language = COALESCE(excluded.language, submissions.language),
               time_ms = COALESCE(excluded.time_ms, submissions.time_ms),
               memory_kb = COALESCE(excluded.memory_kb, submissions.memory_kb),
               raw_json = excluded.raw_json",
        )
        .bind(&seed.id)
        .bind(&seed.platform)
        .bind(&seed.native_id)
        .bind(&seed.problem_id)
        .bind(&seed.verdict)
        .bind(&seed.language)
        .bind(seed.time_ms)
        .bind(seed.memory_kb)
        .bind(&seed.submitted_at)
        .bind(seed.raw.to_string())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn emit_progress(
    app: &AppHandle,
    platform: &str,
    stage: &str,
    message: impl Into<String>,
    percent: f64,
) {
    let _ = app.emit(
        "sync://progress",
        SyncProgressDto {
            platform: platform.to_string(),
            stage: stage.to_string(),
            message: message.into(),
            percent,
            status: "running".into(),
        },
    );
}

fn iso_from_secs(secs: i64) -> String {
    Utc.timestamp_opt(secs, 0)
        .single()
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(now_iso)
}

pub async fn sync_platform(
    app: &AppHandle,
    db: &Db,
    automation: &AutomationManager,
    platform: &str,
) -> AppResult<()> {
    let account = load_account(db, platform).await?;
    match platform {
        "codeforces" => sync_codeforces(app, db, account).await,
        "atcoder" => sync_atcoder(app, db, account).await,
        "luogu" => sync_luogu(app, db, automation, account).await,
        _ => Err(AppError::invalid(format!("未知平台: {platform}"))),
    }
}

pub(crate) async fn http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(crate::llm::APP_UA)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(AppError::network)
}

pub(crate) async fn cf_api(client: &reqwest::Client, path: &str) -> AppResult<Value> {
    // Codeforces asks clients to stay at one request every two seconds.
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    let url = format!("https://codeforces.com/api/{path}");
    let resp = client.get(&url).send().await.map_err(AppError::network)?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::provider(
            "E_PLATFORM_HTTP",
            format!("Codeforces 返回 HTTP {}", status.as_u16()),
        )
        .with_detail(truncate_chars(&text, 500)));
    }
    let value: Value = serde_json::from_str(&text).map_err(|e| {
        AppError::provider("E_PLATFORM_FORMAT", "Codeforces 返回格式异常")
            .with_detail(e.to_string())
    })?;
    if value.get("status").and_then(|v| v.as_str()) != Some("OK") {
        let comment = value
            .get("comment")
            .and_then(|v| v.as_str())
            .unwrap_or("未知错误");
        return Err(AppError::provider(
            "E_PLATFORM_API",
            format!("Codeforces API 错误：{comment}"),
        ));
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

async fn sync_codeforces(app: &AppHandle, db: &Db, account: Option<Account>) -> AppResult<()> {
    let mut account = account.unwrap_or(Account {
        platform: "codeforces".into(),
        handle: None,
        uid: None,
        logged_in: false,
        rating: 0,
        max_rating: 0,
        solved_count: 0,
        meta: json!({}),
        last_synced_at: None,
    });
    let handle = account
        .handle
        .clone()
        .ok_or_else(|| AppError::new("E_PLATFORM_HANDLE", "请先填写 Codeforces 用户名"))?;
    let client = http_client().await?;

    emit_progress(app, "codeforces", "problems", "读取 Codeforces 题池…", 8.0);
    let cached = sqlx::query("SELECT fetched_at FROM platform_cache WHERE key = 'cf:problemset'")
        .fetch_optional(&db.pool)
        .await?;
    let fresh = cached
        .as_ref()
        .and_then(|row| row.get::<Option<String>, _>("fetched_at"))
        .map(|at| {
            chrono::DateTime::parse_from_rfc3339(&at)
                .map(|dt| {
                    Utc::now()
                        .signed_duration_since(dt.with_timezone(&Utc))
                        .num_hours()
                        < 24
                })
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if !fresh {
        let result = cf_api(&client, "problemset.problems").await?;
        let problems = result
            .get("problems")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut seeds = Vec::with_capacity(problems.len());
        for problem in problems {
            let contest_id = match problem.get("contestId").and_then(|v| v.as_i64()) {
                Some(id) => id,
                None => continue,
            };
            let index = problem
                .get("index")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if index.is_empty() {
                continue;
            }
            let native = format!("{contest_id}{index}");
            let tags: Vec<String> = problem
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            seeds.push(ProblemSeed {
                id: format!("cf:{native}"),
                platform: "codeforces".into(),
                native_id: native,
                title: problem
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string(),
                url: format!("https://codeforces.com/problemset/problem/{contest_id}/{index}"),
                rating: problem.get("rating").and_then(|v| v.as_i64()),
                normalized_rating: problem.get("rating").and_then(|v| v.as_i64()),
                tags: tags.clone(),
                skill_tags: skill_tags_for("codeforces", &tags),
                time_limit_ms: Some(2000),
                memory_limit_mb: Some(256),
            });
        }
        upsert_problems(db, &seeds).await?;
        sqlx::query(
            "INSERT INTO platform_cache(key, body, fetched_at) VALUES('cf:problemset', '{}', ?1)
             ON CONFLICT(key) DO UPDATE SET fetched_at = excluded.fetched_at",
        )
        .bind(now_iso())
        .execute(&db.pool)
        .await?;
    }

    emit_progress(app, "codeforces", "profile", "读取账号 rating…", 35.0);
    if let Ok(info) = cf_api(&client, &format!("user.info?handles={handle}")).await {
        if let Some(user) = info.as_array().and_then(|arr| arr.first()) {
            account.rating = user.get("rating").and_then(|v| v.as_i64()).unwrap_or(0);
            account.max_rating = user.get("maxRating").and_then(|v| v.as_i64()).unwrap_or(0);
            if let Some(rank) = user.get("rank").and_then(|v| v.as_str()) {
                account.meta["rank"] = json!(rank);
            }
            if let Some(avatar) = user.get("titlePhoto").and_then(|v| v.as_str()) {
                account.meta["avatar"] = json!(avatar);
            }
        }
    }

    emit_progress(app, "codeforces", "rating", "读取 rating 历史…", 48.0);
    if let Ok(history) = cf_api(&client, &format!("user.rating?handle={handle}")).await {
        let mut points = Vec::new();
        if let Some(items) = history.as_array() {
            for item in items {
                let at = item
                    .get("ratingUpdateTimeSeconds")
                    .and_then(|v| v.as_i64())
                    .map(iso_from_secs);
                let rating = item.get("newRating").and_then(|v| v.as_i64());
                if let (Some(at), Some(rating)) = (at, rating) {
                    points.push(json!({
                        "at": at,
                        "rating": rating,
                        "contest": item.get("contestName").and_then(|v| v.as_str()).unwrap_or("")
                    }));
                }
            }
        }
        account.meta["history"] = Value::Array(points);
    }

    emit_progress(app, "codeforces", "submissions", "读取提交记录…", 62.0);
    let mut seeds: Vec<SubmissionSeed> = Vec::new();
    for page in 1..=3 {
        let from = (page - 1) * 1000 + 1;
        let result = match cf_api(
            &client,
            &format!("user.status?handle={handle}&from={from}&count=1000"),
        )
        .await
        {
            Ok(v) => v,
            Err(err) => {
                if page == 1 {
                    return Err(err);
                }
                break;
            }
        };
        let items = result.as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            break;
        }
        let before = seeds.len();
        for item in items {
            let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
            let problem = match item.get("problem") {
                Some(p) => p,
                None => continue,
            };
            let contest_id = match problem.get("contestId").and_then(|v| v.as_i64()) {
                Some(id) => id,
                None => continue,
            };
            let index = problem
                .get("index")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if index.is_empty() || id == 0 {
                continue;
            }
            let native = format!("{contest_id}{index}");
            let submitted_at = item
                .get("creationTimeSeconds")
                .and_then(|v| v.as_i64())
                .map(iso_from_secs)
                .unwrap_or_else(now_iso);
            seeds.push(SubmissionSeed {
                id: format!("cf:s{id}"),
                platform: "codeforces".into(),
                native_id: id.to_string(),
                problem_id: format!("cf:{native}"),
                verdict: verdict_from_cf(
                    item.get("verdict")
                        .and_then(|v| v.as_str())
                        .unwrap_or("UNKNOWN"),
                ),
                language: item
                    .get("programmingLanguage")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                time_ms: item.get("timeConsumedMillis").and_then(|v| v.as_i64()),
                memory_kb: item
                    .get("memoryConsumedBytes")
                    .and_then(|v| v.as_i64())
                    .map(|b| b / 1024),
                submitted_at,
                raw: item.clone(),
            });
        }
        if seeds.len() == before {
            break;
        }
        emit_progress(
            app,
            "codeforces",
            "submissions",
            format!("已读取 {} 条提交…", seeds.len()),
            62.0 + page as f64 * 8.0,
        );
    }
    upsert_submissions(db, &seeds).await?;

    account.solved_count = count_solved(db, "codeforces").await?;
    account.last_synced_at = Some(now_iso());
    save_account(db, &account).await?;
    emit_progress(app, "codeforces", "done", "Codeforces 同步完成", 100.0);
    Ok(())
}

async fn sync_atcoder(app: &AppHandle, db: &Db, account: Option<Account>) -> AppResult<()> {
    let mut account = account.unwrap_or(Account {
        platform: "atcoder".into(),
        handle: None,
        uid: None,
        logged_in: false,
        rating: 0,
        max_rating: 0,
        solved_count: 0,
        meta: json!({}),
        last_synced_at: None,
    });
    let handle = account
        .handle
        .clone()
        .ok_or_else(|| AppError::new("E_PLATFORM_HANDLE", "请先填写 AtCoder 用户名"))?;
    let client = http_client().await?;

    emit_progress(app, "atcoder", "problems", "读取 AtCoder 题池…", 10.0);
    let resources_fresh = cache_fresh(db, "atcoder:resources", 24).await?;
    if !resources_fresh {
        let problems_resp = client
            .get("https://kenkoooo.com/atcoder/resources/problems.json")
            .send()
            .await
            .map_err(AppError::network)?;
        let problems_text = problems_resp.text().await.unwrap_or_default();
        let problems: Value = serde_json::from_str(&problems_text).map_err(|e| {
            AppError::provider("E_PLATFORM_FORMAT", "AtCoder 题池格式异常")
                .with_detail(e.to_string())
        })?;
        let models_resp = client
            .get("https://kenkoooo.com/atcoder/resources/problem-models.json")
            .send()
            .await
            .map_err(AppError::network)?;
        let models_text = models_resp.text().await.unwrap_or_default();
        let models: Value = serde_json::from_str(&models_text).unwrap_or(json!({}));
        let mut seeds = Vec::new();
        if let Some(items) = problems.as_array() {
            for item in items {
                let id = item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if id.is_empty() {
                    continue;
                }
                let contest_id = item
                    .get("contest_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let difficulty = models
                    .get(&id)
                    .and_then(|v| v.get("difficulty"))
                    .and_then(|v| v.as_f64())
                    .map(|d| d.round() as i64);
                seeds.push(ProblemSeed {
                    id: format!("atcoder:{id}"),
                    platform: "atcoder".into(),
                    native_id: id.clone(),
                    title: item
                        .get("title")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(&id)
                        .to_string(),
                    url: format!("https://atcoder.jp/contests/{contest_id}/tasks/{id}"),
                    rating: difficulty,
                    normalized_rating: difficulty,
                    tags: vec![],
                    skill_tags: vec![],
                    time_limit_ms: Some(2000),
                    memory_limit_mb: Some(1024),
                });
            }
        }
        upsert_problems(db, &seeds).await?;
        set_cache(db, "atcoder:resources", "{}").await?;
    }

    emit_progress(app, "atcoder", "profile", "读取 rating 历史…", 40.0);
    let history_url = format!("https://atcoder.jp/users/{handle}/history/json");
    if let Ok(resp) = client.get(&history_url).send().await {
        if resp.status().is_success() {
            if let Ok(history) = resp.json::<Value>().await {
                let mut points = Vec::new();
                let mut latest = 0i64;
                let mut max_rating = 0i64;
                if let Some(items) = history.as_array() {
                    for item in items {
                        let rating = item.get("NewRating").and_then(|v| v.as_i64());
                        let at = item
                            .get("EndTime")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        if let (Some(rating), Some(at)) = (rating, at) {
                            latest = rating;
                            max_rating = max_rating.max(rating);
                            points.push(json!({
                                "at": at,
                                "rating": rating,
                                "contest": item.get("ContestName").and_then(|v| v.as_str()).unwrap_or("")
                            }));
                        }
                    }
                }
                account.rating = latest;
                account.max_rating = max_rating.max(latest);
                account.meta["history"] = Value::Array(points);
            }
        }
    }

    emit_progress(app, "atcoder", "submissions", "读取提交记录…", 60.0);
    let from = account
        .meta
        .get("last_submission_epoch")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let url = format!(
        "https://kenkoooo.com/atcoder/atcoder-api/v3/user/submissions?user={handle}&from_second={from}"
    );
    let resp = client.get(&url).send().await.map_err(AppError::network)?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::provider(
            "E_PLATFORM_HTTP",
            format!("AtCoder 提交接口返回 HTTP {}", status.as_u16()),
        )
        .with_detail(truncate_chars(&text, 400)));
    }
    let items: Value = serde_json::from_str(&text).map_err(|e| {
        AppError::provider("E_PLATFORM_FORMAT", "AtCoder 提交记录格式异常")
            .with_detail(e.to_string())
    })?;
    let mut seeds = Vec::new();
    let mut max_epoch = from;
    if let Some(arr) = items.as_array() {
        for item in arr {
            let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
            let problem_id = item
                .get("problem_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if id == 0 || problem_id.is_empty() {
                continue;
            }
            let epoch = item
                .get("epoch_second")
                .and_then(|v| v.as_i64())
                .unwrap_or_default();
            max_epoch = max_epoch.max(epoch);
            seeds.push(SubmissionSeed {
                id: format!("atcoder:s{id}"),
                platform: "atcoder".into(),
                native_id: id.to_string(),
                problem_id: format!("atcoder:{problem_id}"),
                verdict: verdict_from_atcoder(
                    item.get("result")
                        .and_then(|v| v.as_str())
                        .unwrap_or("UNKNOWN"),
                ),
                language: item
                    .get("language")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                time_ms: item.get("execution_time").and_then(|v| v.as_i64()),
                memory_kb: None,
                submitted_at: iso_from_secs(epoch),
                raw: item.clone(),
            });
        }
    }
    upsert_submissions(db, &seeds).await?;
    account.meta["last_submission_epoch"] = json!(max_epoch);
    account.solved_count = count_solved(db, "atcoder").await?;
    account.last_synced_at = Some(now_iso());
    save_account(db, &account).await?;
    emit_progress(app, "atcoder", "done", "AtCoder 同步完成", 100.0);
    Ok(())
}

/// Luogu now answers with a Lentille envelope:
/// `{ "data": { "records": { "result": [...], "count", "perPage" } } }`.
/// Older deployments used `body.currentData.records`; accept both, and
/// tolerate `result` being an index map or the list itself being an array.
fn luogu_container(value: &Value) -> &Value {
    let body = value.get("body").unwrap_or(value);
    body.get("data")
        .or_else(|| body.get("currentData"))
        .unwrap_or(body)
}

pub(crate) fn luogu_list_items(value: &Value, key: &str) -> Vec<Value> {
    let body = value.get("body").unwrap_or(value);
    let list = luogu_container(value).get(key).or_else(|| body.get(key));
    let Some(list) = list else {
        return Vec::new();
    };
    let result = list.get("result").unwrap_or(list);
    match result {
        Value::Array(items) => items.clone(),
        Value::Object(map) => map.values().cloned().collect(),
        _ => Vec::new(),
    }
}

fn luogu_record_per_page(value: &Value) -> Option<i64> {
    luogu_container(value)
        .get("records")?
        .get("perPage")?
        .as_i64()
}

pub(crate) fn luogu_data(value: &Value) -> Value {
    luogu_container(value).clone()
}

pub(crate) fn luogu_record_items(value: &Value) -> Vec<Value> {
    luogu_list_items(value, "records")
}

fn luogu_problem_items(value: &Value) -> Vec<Value> {
    luogu_list_items(value, "problems")
}

async fn sync_luogu(
    app: &AppHandle,
    db: &Db,
    automation: &AutomationManager,
    account: Option<Account>,
) -> AppResult<()> {
    let mut account = account.unwrap_or(Account {
        platform: "luogu".into(),
        handle: None,
        uid: None,
        logged_in: false,
        rating: 0,
        max_rating: 0,
        solved_count: 0,
        meta: json!({}),
        last_synced_at: None,
    });
    let uid = account
        .uid
        .clone()
        .ok_or_else(|| AppError::new("E_PLATFORM_HANDLE", "请先填写洛谷 UID"))?;
    let depth = db
        .get_json::<SettingsDto>("settings")
        .await
        .map(|s| s.luogu_sync_depth)
        .unwrap_or(500)
        .clamp(50, 2000);

    emit_progress(app, "luogu", "login", "打开洛谷并检查登录状态…", 5.0);
    automation
        .open(
            app,
            "luogu",
            &format!("https://www.luogu.com.cn/user/{uid}"),
            Some(DEFAULT_BOUNDS),
        )
        .await?;
    automation.wait_ready("luogu", 25_000).await?;
    let challenge = automation
        .eval_json("luogu", challenge_probe(), 5_000)
        .await?;
    if challenge.get("challenge").and_then(|v| v.as_bool()) == Some(true) {
        automation
            .needs_human(
                app,
                "luogu",
                "洛谷触发了人机验证",
                "请在右侧页面完成验证，然后点击“我已处理，继续同步”。",
            )
            .await;
        return Err(AppError::needs_human(
            "洛谷触发了人机验证",
            "请在页面上完成验证后重试同步",
        ));
    }
    if challenge.get("login").and_then(|v| v.as_bool()) == Some(true) {
        automation
            .needs_human(app, "luogu", "洛谷登录已失效", "请在右侧页面登录洛谷。")
            .await;
        return Err(AppError::needs_human("洛谷登录已失效", "请登录后重试"));
    }
    automation
        .set_status(app, "luogu", "running", "读取洛谷用户信息…")
        .await;

    emit_progress(app, "luogu", "profile", "读取洛谷用户信息…", 25.0);
    let user_value = luogu_fetch(app, automation, &format!("/user/{uid}?_contentOnly=1")).await?;
    let data = luogu_data(&user_value);
    let user = data.get("user").cloned().unwrap_or_else(|| data.clone());
    if let Some(name) = user.get("name").and_then(|v| v.as_str()) {
        account.handle = Some(name.to_string());
    }
    if let Some(rating) = user
        .get("rating")
        .or_else(|| user.get("elo"))
        .and_then(|v| v.as_i64())
    {
        account.rating = rating;
        account.max_rating = account.max_rating.max(rating);
    }
    if let Some(passed) = data
        .get("passedProblemCount")
        .or_else(|| user.get("passedProblemCount"))
        .and_then(|v| v.as_i64())
    {
        account.solved_count = passed;
    }

    emit_progress(app, "luogu", "submissions", "读取洛谷提交记录…", 45.0);
    // Tag ids only mean something through `/_lfe/tags`; without it the UI shows
    // `tag-45` instead of 图论/并查集.
    let tag_names = luogu_tag_names(app, automation).await;
    repair_placeholder_tags(app, automation, db, true).await;
    // Lentille returns 20 records per page. Use the actual page size to cover
    // the configured depth instead of assuming a 50-row page.
    let mut per_page = 20i64;
    let mut pages = ((depth + per_page - 1) / per_page).clamp(1, 50);
    let mut page = 1i64;
    let mut seeds = Vec::new();
    let mut code_targets: Vec<(String, String)> = Vec::new();
    while page <= pages {
        let value = match luogu_fetch(
            app,
            automation,
            &format!("/record/list?user={uid}&user_id={uid}&page={page}&_contentOnly=1"),
        )
        .await
        {
            Ok(v) => v,
            Err(err) => {
                if page == 1 {
                    return Err(err);
                }
                break;
            }
        };
        if page == 1 {
            per_page = luogu_record_per_page(&value)
                .unwrap_or(per_page)
                .clamp(1, 100);
            pages = ((depth + per_page - 1) / per_page).clamp(1, 50);
        }
        let records = luogu_record_items(&value);
        if records.is_empty() {
            break;
        }
        for record in records {
            let pid = record
                .get("problem")
                .and_then(|p| p.get("pid"))
                .or_else(|| record.get("pid"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if pid.is_empty() {
                continue;
            }
            let status = record.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
            let submit_time = record
                .get("submitTime")
                .or_else(|| record.get("time"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let submitted_at = if submit_time > 1_000_000_000_000 {
                iso_from_secs(submit_time / 1000)
            } else {
                iso_from_secs(submit_time)
            };
            let id = record
                .get("id")
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string())
                .or_else(|| {
                    record
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| format!("{pid}-{submit_time}"));
            let title = record
                .get("problem")
                .and_then(|p| p.get("title").or_else(|| p.get("name")))
                .and_then(|v| v.as_str())
                .unwrap_or(&pid)
                .to_string();
            let difficulty = record
                .get("problem")
                .and_then(|p| p.get("difficulty"))
                .and_then(|v| v.as_i64());
            if code_targets.len() < 20 {
                code_targets.push((format!("luogu:s{id}"), id.clone()));
            }
            seeds.push(SubmissionSeed {
                id: format!("luogu:s{id}"),
                platform: "luogu".into(),
                native_id: id,
                problem_id: format!("luogu:{pid}"),
                verdict: verdict_from_luogu(status),
                language: record
                    .get("language")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                time_ms: record.get("time").and_then(|v| v.as_i64()),
                memory_kb: record.get("memory").and_then(|v| v.as_i64()),
                submitted_at,
                raw: record.clone(),
            });
            if !title.is_empty() {
                let tags = record
                    .get("problem")
                    .map(|problem| luogu_tags(problem, &tag_names))
                    .unwrap_or_default();
                let _ = upsert_problems(
                    db,
                    &[ProblemSeed {
                        id: format!("luogu:{pid}"),
                        platform: "luogu".into(),
                        native_id: pid.clone(),
                        title,
                        url: format!("https://www.luogu.com.cn/problem/{pid}"),
                        rating: difficulty,
                        normalized_rating: luogu_normalized(difficulty),
                        tags: tags.clone(),
                        skill_tags: skill_tags_for("luogu", &tags),
                        time_limit_ms: None,
                        memory_limit_mb: None,
                    }],
                )
                .await;
            }
        }
        emit_progress(
            app,
            "luogu",
            "submissions",
            format!("已读取第 {page} 页提交…"),
            45.0 + (page as f64 / pages as f64) * 30.0,
        );
        if page < pages {
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        page += 1;
    }
    upsert_submissions(db, &seeds).await?;

    if !code_targets.is_empty() {
        emit_progress(app, "luogu", "code", "读取最近提交的代码…", 78.0);
        for (submission_id, record_id) in &code_targets {
            if let Ok(value) = luogu_fetch(
                app,
                automation,
                &format!("/record/{record_id}?_contentOnly=1"),
            )
            .await
            {
                let data = luogu_data(&value);
                let code = data
                    .get("record")
                    .and_then(|record| record.get("sourceCode"))
                    .and_then(|value| value.as_str());
                if let Some(code) = code.filter(|code| !code.trim().is_empty()) {
                    let _ = sqlx::query(
                        "INSERT INTO submission_code(submission_id, code_text) VALUES(?1, ?2)
                         ON CONFLICT(submission_id) DO UPDATE SET code_text = excluded.code_text",
                    )
                    .bind(submission_id)
                    .bind(code)
                    .execute(&db.pool)
                    .await;
                }
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
    }

    emit_progress(app, "luogu", "candidates", "读取洛谷题目池…", 82.0);
    for page in 1..=3 {
        let value = match luogu_fetch(
            app,
            automation,
            &format!("/problem/list?page={page}&_contentOnly=1"),
        )
        .await
        {
            Ok(v) => v,
            Err(_) => break,
        };
        let problems = luogu_problem_items(&value);
        if problems.is_empty() {
            break;
        }
        let mut pool = Vec::new();
        for problem in problems {
            let pid = problem
                .get("pid")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if pid.is_empty() {
                continue;
            }
            let difficulty = problem.get("difficulty").and_then(|v| v.as_i64());
            let tags = luogu_tags(&problem, &tag_names);
            pool.push(ProblemSeed {
                id: format!("luogu:{pid}"),
                platform: "luogu".into(),
                native_id: pid.clone(),
                title: problem
                    .get("title")
                    .or_else(|| problem.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&pid)
                    .to_string(),
                url: format!("https://www.luogu.com.cn/problem/{pid}"),
                rating: difficulty,
                normalized_rating: luogu_normalized(difficulty),
                tags: tags.clone(),
                skill_tags: skill_tags_for("luogu", &tags),
                time_limit_ms: None,
                memory_limit_mb: None,
            });
        }
        upsert_problems(db, &pool).await?;
    }

    account.solved_count = count_solved(db, "luogu").await?.max(account.solved_count);
    account.logged_in = true;
    account.last_synced_at = Some(now_iso());
    save_account(db, &account).await?;
    automation
        .set_status(app, "luogu", "done", "洛谷同步完成")
        .await;
    emit_progress(app, "luogu", "done", "洛谷同步完成", 100.0);
    Ok(())
}

async fn count_solved(db: &Db, platform: &str) -> AppResult<i64> {
    let row = sqlx::query(
        "SELECT COUNT(DISTINCT problem_id) AS n FROM submissions WHERE platform = ?1 AND verdict = 'AC'",
    )
    .bind(platform)
    .fetch_one(&db.pool)
    .await?;
    Ok(row.get::<i64, _>("n"))
}

async fn cache_fresh(db: &Db, key: &str, hours: i64) -> AppResult<bool> {
    let row = sqlx::query("SELECT fetched_at FROM platform_cache WHERE key = ?1")
        .bind(key)
        .fetch_optional(&db.pool)
        .await?;
    Ok(row
        .and_then(|r| r.get::<Option<String>, _>("fetched_at"))
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(&at).ok())
        .map(|dt| {
            Utc::now()
                .signed_duration_since(dt.with_timezone(&Utc))
                .num_hours()
                < hours
        })
        .unwrap_or(false))
}

async fn set_cache(db: &Db, key: &str, body: &str) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO platform_cache(key, body, fetched_at) VALUES(?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET body = excluded.body, fetched_at = excluded.fetched_at",
    )
    .bind(key)
    .bind(body)
    .bind(now_iso())
    .execute(&db.pool)
    .await?;
    Ok(())
}

async fn luogu_fetch(
    app: &AppHandle,
    automation: &AutomationManager,
    path: &str,
) -> AppResult<Value> {
    let value = match automation.fetch_json_in_page("luogu", path, 30_000).await {
        Ok(v) => v,
        Err(err) => {
            if err.code == "E_AUTOMATION_TIMEOUT" {
                automation
                    .needs_human(
                        app,
                        "luogu",
                        "洛谷没有返回数据",
                        "页面可能触发了验证或登录已失效，请在右侧页面处理后重试。",
                    )
                    .await;
            }
            return Err(err);
        }
    };
    let status = value.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
    if status == 0 {
        let err = value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("网络错误");
        return Err(AppError::network(format!("洛谷页面请求失败: {err}")));
    }
    let body = value.get("body");
    let has_json_envelope = body
        .and_then(|b| b.get("data").or_else(|| b.get("currentData")))
        .is_some();
    if body.and_then(|b| b.get("__raw")).is_some() && !has_json_envelope {
        return Err(AppError::network(
            "洛谷返回了网页而不是 JSON，请确认登录状态后重试",
        ));
    }
    Ok(value)
}

pub fn luogu_normalized(difficulty: Option<i64>) -> Option<i64> {
    difficulty.map(|d| match d.clamp(1, 7) {
        1 => 800,
        2 => 1200,
        3 => 1500,
        4 => 1800,
        5 => 2100,
        6 => 2400,
        _ => 2800,
    })
}

pub fn skill_tags_for(platform: &str, tags: &[String]) -> Vec<String> {
    let _ = platform;
    tags.iter()
        .map(|tag| map_skill_tag(tag))
        // Tags outside the ASCII dictionary used to collapse to "" and would
        // then create a bogus skill profile keyed by the empty string.
        .filter(|tag| !tag.is_empty())
        .collect()
}

fn map_skill_tag(tag: &str) -> String {
    let lower = tag.to_ascii_lowercase();
    match lower.as_str() {
        "dp" => "dp".to_string(),
        "graphs" => "graphs".to_string(),
        "data structures" => "data-structures".to_string(),
        "greedy" => "greedy".to_string(),
        "math" => "math".to_string(),
        "implementation" => "implementation".to_string(),
        "strings" => "strings".to_string(),
        "binary search" => "binary-search".to_string(),
        "sortings" => "sorting".to_string(),
        "trees" => "trees".to_string(),
        "geometry" => "geometry".to_string(),
        "number theory" => "number-theory".to_string(),
        "combinatorics" => "combinatorics".to_string(),
        "brute force" => "brute-force".to_string(),
        "constructive algorithms" => "constructive".to_string(),
        "dfs and similar" => "dfs".to_string(),
        "shortest paths" => "shortest-paths".to_string(),
        "two pointers" => "two-pointers".to_string(),
        "bitmasks" => "bitmasks".to_string(),
        "dsu" => "dsu".to_string(),
        "flows" => "flows".to_string(),
        "games" => "games".to_string(),
        "probabilities" => "probability".to_string(),
        "interactive" => "interactive".to_string(),
        "divide and conquer" => "divide-and-conquer".to_string(),
        "hashing" => "hashing".to_string(),
        "matrices" => "matrix".to_string(),
        "string suffix structures" => "string-structures".to_string(),
        "ternary search" => "ternary-search".to_string(),
        "meet-in-the-middle" => "meet-in-the-middle".to_string(),
        "schedules" => "scheduling".to_string(),
        "graph matchings" => "matching".to_string(),
        other => canonical_tag(other),
    }
}

fn canonical_tag(tag: &str) -> String {
    tag.trim()
        .to_ascii_lowercase()
        .replace([' ', '_'], "-")
        // Keep non-ASCII letters: Luogu tag names are Chinese, and the old
        // ASCII-only filter turned every one of them into an empty tag.
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}

pub async fn check_handle(platform: &str, handle: &str) -> AppResult<String> {
    let handle = handle.trim();
    if handle.is_empty() {
        return Err(AppError::invalid("请先填写 handle"));
    }
    match platform {
        "codeforces" => {
            let client = http_client().await?;
            let value = cf_api(&client, &format!("user.info?handles={handle}")).await?;
            let user = value
                .get("result")
                .and_then(|v| v.as_array())
                .and_then(|items| items.first());
            let actual = user
                .and_then(|u| u.get("handle"))
                .and_then(|v| v.as_str())
                .unwrap_or(handle);
            let rating = user
                .and_then(|u| u.get("rating"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            Ok(format!("handle 有效：{actual}，当前 rating {rating}"))
        }
        _ => Err(AppError::invalid(
            "该校验目前只支持 Codeforces；请使用登录验证",
        )),
    }
}

pub async fn verify_login(
    app: &AppHandle,
    db: &Db,
    automation: &AutomationManager,
    platform: &str,
) -> AppResult<crate::models::PlatformLoginResult> {
    automation
        .open(app, platform, login_url(platform), Some(DEFAULT_BOUNDS))
        .await?;
    automation.wait_ready(platform, 25_000).await?;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let challenge = automation
        .eval_json(platform, challenge_probe(), 5_000)
        .await?;
    if challenge.get("challenge").and_then(|v| v.as_bool()) == Some(true) {
        let hint = if platform == "codeforces" {
            "Codeforces 数据同步不需要登录：请关闭面板，在设置中填写 handle 后直接点“同步”。自动提交可能被 Cloudflare 拦截；可在系统浏览器手动提交后，用“手动录入提交结果”记录。"
        } else {
            "请在右侧页面完成人机验证后重试。"
        };
        automation
            .needs_human(app, platform, "页面触发了 Cloudflare/人机验证", hint)
            .await;
        return Err(AppError::needs_human(
            "页面触发了 Cloudflare/人机验证",
            hint,
        ));
    }
    let probe = automation
        .eval_json(platform, login_probe(platform), 8_000)
        .await?;
    let logged = probe
        .get("logged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let handle = probe
        .get("handle")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let uid = probe
        .get("uid")
        .and_then(|v| v.as_i64())
        .map(|v| v.to_string());
    let mut account = load_account(db, platform).await?.unwrap_or(Account {
        platform: platform.to_string(),
        handle: None,
        uid: None,
        logged_in: false,
        rating: 0,
        max_rating: 0,
        solved_count: 0,
        meta: json!({}),
        last_synced_at: None,
    });
    if logged {
        account.logged_in = true;
        if handle.is_some() {
            account.handle = handle.clone();
        }
        if uid.is_some() {
            account.uid = uid.clone();
        }
        account.last_synced_at = account.last_synced_at.clone();
        save_account(db, &account).await?;
    } else {
        account.logged_in = false;
        save_account(db, &account).await?;
    }
    Ok(crate::models::PlatformLoginResult {
        logged_in: logged,
        handle: handle.or(account.handle),
        message: if logged {
            "已检测到登录状态".into()
        } else {
            "还没有检测到登录状态，请在右侧页面完成登录后重试".into()
        },
    })
}

pub async fn fetch_problem(
    app: &AppHandle,
    db: &Db,
    automation: &AutomationManager,
    problem_id: &str,
    refresh: bool,
) -> AppResult<ProblemDto> {
    let row = sqlx::query("SELECT * FROM problems WHERE id = ?1")
        .bind(problem_id)
        .fetch_optional(&db.pool)
        .await?
        .ok_or_else(|| AppError::not_found("题目"))?;
    let platform: String = row.get("platform");
    let native_id: String = row.get("native_id");
    let current_statement: Option<String> = row.get("statement_md");
    let tags_json: String = row.get("tags_json");
    let empty = current_statement.as_deref().unwrap_or("").trim().is_empty();
    // Older Luogu rows kept the rendered page instead of the statement fields,
    // and their tags are still `tag-<id>`; heal them on open.
    let stale = platform == "luogu"
        && luogu_statement_needs_refresh(current_statement.as_deref(), &tags_json);
    if refresh || empty || stale {
        let outcome = match platform.as_str() {
            "codeforces" => fetch_codeforces_statement(app, db, problem_id, &native_id).await,
            "atcoder" => fetch_atcoder_statement(app, db, problem_id, &native_id).await,
            "luogu" => fetch_luogu_statement(app, db, automation, problem_id, &native_id).await,
            _ => Ok(()),
        };
        if let Err(err) = outcome {
            // An automatic heal must not turn a readable (if stale) statement
            // into an error page; an explicit refresh or a missing statement
            // still reports the failure.
            if refresh || empty {
                return Err(err);
            }
            tracing::warn!("rebuild statement failed for {problem_id}: {}", err.message);
        }
    }
    problem_dto(db, problem_id).await
}

async fn fetch_codeforces_statement(
    _app: &AppHandle,
    db: &Db,
    problem_id: &str,
    native_id: &str,
) -> AppResult<()> {
    let client = http_client().await?;
    let split = native_id
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(native_id.len());
    let contest = &native_id[..split];
    let index = &native_id[split..];
    if contest.is_empty() || index.is_empty() {
        return Err(AppError::invalid("无法解析 Codeforces 题目编号"));
    }
    let url = format!("https://codeforces.com/problemset/problem/{contest}/{index}");
    let html = client
        .get(&url)
        .send()
        .await
        .map_err(AppError::network)?
        .text()
        .await
        .unwrap_or_default();
    let (text, samples, time_limit, memory_limit) = {
        let doc = Html::parse_document(&html);
        let statement_sel = Selector::parse(".problem-statement").unwrap();
        let statement = doc.select(&statement_sel).next().ok_or_else(|| {
            AppError::provider(
                "E_PLATFORM_FORMAT",
                "无法解析 Codeforces 题面，页面结构可能已变化",
            )
        })?;
        let text = html_to_text(&statement.inner_html());
        let (time_limit, memory_limit) = parse_limits(&text);
        let input_sel = Selector::parse(".sample-test .input pre").unwrap();
        let output_sel = Selector::parse(".sample-test .output pre").unwrap();
        let inputs: Vec<String> = doc
            .select(&input_sel)
            .map(|el| el.text().collect::<String>().trim().to_string())
            .collect();
        let outputs: Vec<String> = doc
            .select(&output_sel)
            .map(|el| el.text().collect::<String>().trim().to_string())
            .collect();
        let mut samples = Vec::new();
        for i in 0..inputs.len().min(outputs.len()) {
            samples.push(SampleDto {
                input: inputs[i].clone(),
                output: outputs[i].clone(),
            });
        }
        (text, samples, time_limit, memory_limit)
    };
    sqlx::query(
        "UPDATE problems SET statement_md = ?1, samples_json = ?2, time_limit_ms = COALESCE(?3, time_limit_ms), memory_limit_mb = COALESCE(?4, memory_limit_mb), fetched_at = ?5, content_hash = ?6 WHERE id = ?7",
    )
    .bind(&text)
    .bind(serde_json::to_string(&samples).unwrap_or_else(|_| "[]".into()))
    .bind(time_limit)
    .bind(memory_limit)
    .bind(now_iso())
    .bind(crate::util::sha256_hex(&text))
    .bind(problem_id)
    .execute(&db.pool)
    .await?;
    Ok(())
}

async fn fetch_atcoder_statement(
    _app: &AppHandle,
    db: &Db,
    problem_id: &str,
    native_id: &str,
) -> AppResult<()> {
    let _ = native_id;
    let client = http_client().await?;
    let row = sqlx::query("SELECT url FROM problems WHERE id = ?1")
        .bind(problem_id)
        .fetch_one(&db.pool)
        .await?;
    let url: String = row.get("url");
    let html = client
        .get(&url)
        .send()
        .await
        .map_err(AppError::network)?
        .text()
        .await
        .unwrap_or_default();
    let (text, samples, time_limit, memory_limit) = {
        let doc = Html::parse_document(&html);
        let task_sel = Selector::parse("#task-statement").unwrap();
        let task = doc.select(&task_sel).next().ok_or_else(|| {
            AppError::provider(
                "E_PLATFORM_FORMAT",
                "无法解析 AtCoder 题面，页面结构可能已变化",
            )
        })?;
        let lang_sel = Selector::parse(".lang-en").unwrap();
        let english = doc
            .select(&lang_sel)
            .next()
            .map(|el| el.inner_html())
            .unwrap_or_else(|| task.inner_html());
        let text = html_to_text(&english);
        let body_text = doc.root_element().text().collect::<String>();
        let (time_limit, memory_limit) = parse_limits(&body_text);
        let pre_sel = Selector::parse("pre").unwrap();
        let pres: Vec<String> = if english.contains("<pre") {
            let fragment = Html::parse_fragment(&english);
            fragment
                .select(&pre_sel)
                .map(|el| el.text().collect::<String>().trim().to_string())
                .collect()
        } else {
            task.select(&pre_sel)
                .map(|el| el.text().collect::<String>().trim().to_string())
                .collect()
        };
        let mut samples = Vec::new();
        for pair in pres.chunks(2) {
            if pair.len() == 2 {
                samples.push(SampleDto {
                    input: pair[0].clone(),
                    output: pair[1].clone(),
                });
            }
        }
        (text, samples, time_limit, memory_limit)
    };
    sqlx::query(
        "UPDATE problems SET statement_md = ?1, samples_json = ?2, time_limit_ms = COALESCE(?3, time_limit_ms), memory_limit_mb = COALESCE(?4, memory_limit_mb), fetched_at = ?5, content_hash = ?6 WHERE id = ?7",
    )
    .bind(&text)
    .bind(serde_json::to_string(&samples).unwrap_or_else(|_| "[]".into()))
    .bind(time_limit)
    .bind(memory_limit)
    .bind(now_iso())
    .bind(crate::util::sha256_hex(&text))
    .bind(problem_id)
    .execute(&db.pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Luogu statement, limits and tags
//
// `GET /problem/{pid}?_contentOnly=1` returns the statement as separate
// markdown fields. Scraping the rendered page instead picks up the whole Vue
// shell (navigation, buttons, sample boxes) with the formulas and structure
// lost, which is why the editor used to show one long paragraph.
// ---------------------------------------------------------------------------

fn inline_html_to_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find('>') else {
            out.push_str(tail);
            return out;
        };
        let tag = &tail[..=end];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<img") {
            if let Some(src) = attribute_value(tag, "src") {
                out.push_str(&format!("![]({src})"));
            }
        } else if lower.starts_with("<br") {
            out.push('\n');
        } else {
            out.push_str(tag);
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn attribute_value(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let index = lower.find(&format!("{name}="))?;
    let after = &tag[index + name.len() + 1..];
    let quote = after.chars().next()?;
    if quote == '"' || quote == '\'' {
        let value = &after[1..];
        let end = value.find(quote)?;
        Some(value[..end].trim().to_string())
    } else {
        let end = after
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(after.len());
        Some(after[..end].trim().to_string())
    }
}

/// Headings follow the order Luogu itself renders.
///
/// The fields live under `problem.content` as `background` / `description` /
/// `formatI` / `formatO` / `hint`; older exports also carried them at the top
/// level as `inputFormat` / `outputFormat`, so both shapes are accepted.
pub fn luogu_statement_markdown(problem: &Value) -> String {
    let content = problem
        .get("content")
        .filter(|value| value.is_object())
        .unwrap_or(problem);
    let field = |keys: &[&str]| {
        keys.iter().find_map(|key| {
            content
                .get(*key)
                .or_else(|| problem.get(*key))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
        })
    };
    let mut out = String::new();
    for (heading, keys) in [
        ("题目背景", &["background"][..]),
        ("题目描述", &["description"]),
        ("输入格式", &["formatI", "inputFormat"]),
        ("输出格式", &["formatO", "outputFormat"]),
        ("说明/提示", &["hint"]),
    ] {
        if let Some(text) = field(keys) {
            out.push_str(&format!(
                "## {heading}\n\n{}\n\n",
                inline_html_to_markdown(text)
            ));
        }
    }
    out.trim_end().to_string()
}

/// Returns `(time_limit_ms, memory_limit_mb)`.
pub fn luogu_limits(problem: &Value) -> (Option<i64>, Option<i64>) {
    let max_of = |key: &str| {
        problem
            .get("limits")
            .and_then(|limits| limits.get(key))
            .and_then(|value| value.as_array())
            .map(|items| items.iter().filter_map(|item| item.as_i64()).max())
            .unwrap_or(None)
            .filter(|value| *value > 0)
    };
    let time = max_of("time");
    // Luogu reports KiB (125000 for "125 MB"); very old exports used MiB.
    let memory = max_of("memory").map(|kb| if kb >= 1024 { kb / 1024 } else { kb });
    (time, memory)
}

/// Maps the numeric tag ids Luogu returns to the names shown in its UI.
pub fn luogu_tags(problem: &Value, names: &HashMap<i64, String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Some(tags) = problem.get("tags").and_then(|value| value.as_array()) else {
        return out;
    };
    for tag in tags {
        let name = tag
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| tag.as_str().map(str::to_string))
            .or_else(|| tag.as_i64().and_then(|id| names.get(&id).cloned()))
            .or_else(|| tag.as_i64().map(|id| format!("tag-{id}")));
        if let Some(name) = name {
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// `(fetched_at, id → name)`; refreshed hourly.
type LuoguTagCache = Mutex<Option<(Instant, HashMap<i64, String>)>>;

fn luogu_tag_cache() -> &'static LuoguTagCache {
    static CACHE: OnceLock<LuoguTagCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

async fn luogu_tag_names(app: &AppHandle, automation: &AutomationManager) -> HashMap<i64, String> {
    const TTL: Duration = Duration::from_secs(3_600);
    if let Ok(guard) = luogu_tag_cache().lock() {
        if let Some((at, names)) = guard.as_ref() {
            if at.elapsed() < TTL {
                return names.clone();
            }
        }
    }
    let mut names = HashMap::new();
    if let Ok(value) = luogu_fetch(app, automation, "/_lfe/tags").await {
        let data = luogu_data(&value);
        if let Some(tags) = data.get("tags").and_then(|value| value.as_array()) {
            for tag in tags {
                if let (Some(id), Some(name)) = (
                    tag.get("id").and_then(|value| value.as_i64()),
                    tag.get("name").and_then(|value| value.as_str()),
                ) {
                    names.insert(id, name.to_string());
                }
            }
        }
    }
    if !names.is_empty() {
        if let Ok(mut guard) = luogu_tag_cache().lock() {
            *guard = Some((Instant::now(), names.clone()));
        }
    }
    names
}

/// Rewrites every stored `tag-<id>` placeholder to the real tag name.
///
/// Luogu tag ids are meaningless in the UI (the dashboard used to render
/// `tag-45`), and rows written before the id→name mapping existed stay in the
/// database forever. Skill profile rows are merged by attempts so the Elo does
/// not get lost when both `tag-45` and `二分` already exist.
pub async fn tidy_placeholder_tags(db: &Db, names: &HashMap<i64, String>) -> AppResult<usize> {
    if names.is_empty() {
        return Ok(0);
    }
    let map_tag = |tag: &str| -> Option<String> {
        tag.strip_prefix("tag-")
            .and_then(|id| id.parse::<i64>().ok())
            .and_then(|id| names.get(&id))
            .cloned()
    };
    let mut changed = 0usize;
    for (table, column) in [
        ("problems", "tags_json"),
        ("problems", "skill_tags_json"),
        ("plan_items", "target_tags_json"),
    ] {
        let rows = sqlx::query(&format!(
            "SELECT rowid AS rid, {column} AS value FROM {table} WHERE {column} LIKE '%tag-%'"
        ))
        .fetch_all(&db.pool)
        .await?;
        for row in rows {
            let raw: String = row.get("value");
            let Ok(tags) = serde_json::from_str::<Vec<String>>(&raw) else {
                continue;
            };
            let mut mapped: Vec<String> = Vec::with_capacity(tags.len());
            for tag in &tags {
                let next = map_tag(tag).unwrap_or_else(|| tag.clone());
                if !mapped.contains(&next) {
                    mapped.push(next);
                }
            }
            if mapped != tags {
                changed += tags.iter().filter(|tag| map_tag(tag).is_some()).count();
                sqlx::query(&format!(
                    "UPDATE {table} SET {column} = ?1 WHERE rowid = ?2"
                ))
                .bind(serde_json::to_string(&mapped).unwrap_or(raw))
                .bind(row.get::<i64, _>("rid"))
                .execute(&db.pool)
                .await?;
            }
        }
    }
    let rows = sqlx::query(
        "SELECT tag, rating, attempts, solved FROM skill_profiles WHERE tag LIKE 'tag-%'",
    )
    .fetch_all(&db.pool)
    .await?;
    for row in rows {
        let tag: String = row.get("tag");
        let Some(name) = map_tag(&tag) else {
            continue;
        };
        let rating: f64 = row.get("rating");
        let attempts: i64 = row.get("attempts");
        let solved: i64 = row.get("solved");
        let existing =
            sqlx::query("SELECT rating, attempts, solved FROM skill_profiles WHERE tag = ?1")
                .bind(&name)
                .fetch_optional(&db.pool)
                .await?;
        match existing {
            Some(existing) => {
                let other_rating: f64 = existing.get("rating");
                let other_attempts: i64 = existing.get("attempts");
                let other_solved: i64 = existing.get("solved");
                let total = (attempts + other_attempts).max(1) as f64;
                let merged =
                    (rating * attempts as f64 + other_rating * other_attempts as f64) / total;
                sqlx::query(
                    "UPDATE skill_profiles SET rating = ?1, attempts = ?2, solved = ?3, updated_at = ?4 WHERE tag = ?5",
                )
                .bind(merged)
                .bind(attempts + other_attempts)
                .bind(solved + other_solved)
                .bind(now_iso())
                .bind(&name)
                .execute(&db.pool)
                .await?;
            }
            None => {
                sqlx::query("UPDATE skill_profiles SET tag = ?1, updated_at = ?2 WHERE tag = ?3")
                    .bind(&name)
                    .bind(now_iso())
                    .bind(&tag)
                    .execute(&db.pool)
                    .await?;
            }
        }
        sqlx::query("DELETE FROM skill_profiles WHERE tag = ?1")
            .bind(&tag)
            .execute(&db.pool)
            .await?;
        changed += 1;
    }
    Ok(changed)
}

/// Runs the placeholder repair once per process. `allow_fetch` controls whether
/// the Luogu tag dictionary may be fetched through the automation webview: at
/// startup only the dictionary cached in `luogu.tags` is used, so launching the
/// app never opens a browser window on its own.
pub async fn repair_placeholder_tags(
    app: &AppHandle,
    automation: &AutomationManager,
    db: &Db,
    allow_fetch: bool,
) {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut names: HashMap<i64, String> = db.get_json("luogu.tags").await.unwrap_or_default();
    if names.is_empty() && allow_fetch {
        names = luogu_tag_names(app, automation).await;
        if !names.is_empty() {
            let _ = db.set_json("luogu.tags", &names).await;
        }
    }
    if names.is_empty() {
        return;
    }
    match tidy_placeholder_tags(db, &names).await {
        Ok(0) => {}
        Ok(count) => tracing::info!("rewrote {count} luogu tag placeholders"),
        Err(err) => tracing::warn!("tag placeholder repair failed: {}", err.message),
    }
    DONE.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// True when the stored statement is really the rendered page (the old
/// scraper's output) or when the tags still carry raw `tag-<id>` placeholders.
pub fn luogu_statement_needs_refresh(statement: Option<&str>, tags_json: &str) -> bool {
    let chrome = ["个人中心", "题目列表", "提交答案", "加入题单", "关于洛谷"];
    let text = statement.unwrap_or("").trim();
    if text.is_empty() {
        return true;
    }
    let hits = chrome
        .iter()
        .filter(|marker| text.contains(**marker))
        .count();
    hits >= 2 || tags_json.contains("\"tag-")
}

async fn fetch_luogu_statement(
    app: &AppHandle,
    db: &Db,
    automation: &AutomationManager,
    problem_id: &str,
    native_id: &str,
) -> AppResult<()> {
    let url = format!("https://www.luogu.com.cn/problem/{native_id}");
    automation
        .open(app, "luogu", &url, Some(DEFAULT_BOUNDS))
        .await?;
    automation.wait_ready("luogu", 25_000).await?;
    let value = luogu_fetch(
        app,
        automation,
        &format!("/problem/{native_id}?_contentOnly=1"),
    )
    .await?;
    let problem = luogu_data(&value)
        .get("problem")
        .cloned()
        .unwrap_or(Value::Null);
    let names = luogu_tag_names(app, automation).await;
    // Opening any problem is a cheap moment to persist the tag dictionary and
    // fix rows written before the id→name mapping existed.
    repair_placeholder_tags(app, automation, db, true).await;

    let markdown = luogu_statement_markdown(&problem);
    let statement = if markdown.trim().is_empty() {
        // Last resort only: a translated problem may expose its body elsewhere.
        let fallback = automation
            .eval_json(
                "luogu",
                r#"(() => {
  const node = document.querySelector('.problem-content, article');
  return JSON.stringify({ text: (node ? node.innerText : '').slice(0, 60000) });
})()"#,
                8_000,
            )
            .await?;
        fallback
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        markdown
    };

    let mut samples = Vec::new();
    if let Some(arr) = problem.get("samples").and_then(|v| v.as_array()) {
        for sample in arr {
            if let Some(pair) = sample.as_array() {
                if pair.len() >= 2 {
                    samples.push(SampleDto {
                        input: pair[0].as_str().unwrap_or("").to_string(),
                        output: pair[1].as_str().unwrap_or("").to_string(),
                    });
                }
            }
        }
    }
    let tags = luogu_tags(&problem, &names);
    let skill_tags = skill_tags_for("luogu", &tags);
    let (time_limit_ms, memory_limit_mb) = luogu_limits(&problem);
    let difficulty = problem.get("difficulty").and_then(|v| v.as_i64());
    sqlx::query(
        "UPDATE problems SET statement_md = ?1, samples_json = ?2,
            tags_json = ?3, skill_tags_json = ?4,
            time_limit_ms = COALESCE(?5, time_limit_ms),
            memory_limit_mb = COALESCE(?6, memory_limit_mb),
            normalized_rating = COALESCE(?7, normalized_rating),
            rating = COALESCE(?8, rating),
            fetched_at = ?9, content_hash = ?10
         WHERE id = ?11",
    )
    .bind(&statement)
    .bind(serde_json::to_string(&samples).unwrap_or_else(|_| "[]".into()))
    .bind(serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into()))
    .bind(serde_json::to_string(&skill_tags).unwrap_or_else(|_| "[]".into()))
    .bind(time_limit_ms)
    .bind(memory_limit_mb)
    .bind(luogu_normalized(difficulty))
    .bind(difficulty)
    .bind(now_iso())
    .bind(crate::util::sha256_hex(&statement))
    .bind(problem_id)
    .execute(&db.pool)
    .await?;
    Ok(())
}

pub async fn problem_dto(db: &Db, problem_id: &str) -> AppResult<ProblemDto> {
    let row = sqlx::query(
        "SELECT p.*,
            EXISTS(SELECT 1 FROM submissions s WHERE s.problem_id = p.id AND s.verdict = 'AC') AS solved,
            EXISTS(SELECT 1 FROM submissions s WHERE s.problem_id = p.id) AS attempted
         FROM problems p WHERE p.id = ?1",
    )
    .bind(problem_id)
    .fetch_optional(&db.pool)
    .await?
    .ok_or_else(|| AppError::not_found("题目"))?;
    let samples: Vec<SampleDto> =
        serde_json::from_str(row.get::<&str, _>("samples_json")).unwrap_or_default();
    Ok(ProblemDto {
        id: row.get("id"),
        platform: row.get("platform"),
        native_id: row.get("native_id"),
        title: row.get("title"),
        url: row.get("url"),
        rating: row.get("rating"),
        normalized_rating: row.get("normalized_rating"),
        tags: crate::util::parse_json_array(Some(row.get::<&str, _>("tags_json").to_string())),
        skill_tags: crate::util::parse_json_array(Some(
            row.get::<&str, _>("skill_tags_json").to_string(),
        )),
        time_limit_ms: row.get("time_limit_ms"),
        memory_limit_mb: row.get("memory_limit_mb"),
        statement_md: row
            .get::<Option<String>, _>("statement_md")
            .unwrap_or_default(),
        samples,
        solved: row.get::<i64, _>("solved") != 0,
        attempted: row.get::<i64, _>("attempted") != 0,
    })
}

fn parse_limits(text: &str) -> (Option<i64>, Option<i64>) {
    let time_re =
        Regex::new(r"(?i)time limit[^0-9]{0,20}(\d+(?:\.\d+)?)\s*(seconds?|sec)").unwrap();
    let mem_re = Regex::new(r"(?i)memory limit[^0-9]{0,20}(\d+)\s*(megabytes?|MB)").unwrap();
    let time = time_re
        .captures(text)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())
        .map(|s| (s * 1000.0).round() as i64);
    let memory = mem_re
        .captures(text)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<i64>().ok());
    (time, memory)
}

pub async fn load_problem_samples(db: &Db, problem_id: &str) -> AppResult<Vec<SampleDto>> {
    let row = sqlx::query("SELECT samples_json FROM problems WHERE id = ?1")
        .bind(problem_id)
        .fetch_optional(&db.pool)
        .await?
        .ok_or_else(|| AppError::not_found("题目"))?;
    Ok(serde_json::from_str(row.get::<&str, _>("samples_json")).unwrap_or_default())
}

pub fn platform_dto(platform: &str, account: Option<&Account>) -> crate::models::PlatformDto {
    let logged_in = account.map(|a| a.logged_in).unwrap_or(false);
    let handle = account.and_then(|a| a.handle.clone());
    crate::models::PlatformDto {
        id: platform.to_string(),
        name: platform_name(platform).to_string(),
        description: match platform {
            "codeforces" => "官方公开 API 读取 rating 与提交，题面/提交走内置浏览器".into(),
            "atcoder" => "kenkoooo 公开数据读取比赛与提交，提交走内置浏览器".into(),
            "luogu" => "登录后读取洛谷个人数据；反爬较强，必要时人工接管".into(),
            _ => String::new(),
        },
        login_url: login_url(platform).to_string(),
        needs_handle: true,
        handle_label: match platform {
            "codeforces" => "Codeforces handle".into(),
            "atcoder" => "AtCoder 用户名".into(),
            "luogu" => "洛谷 UID".into(),
            _ => "账号".into(),
        },
        logged_in,
        handle: handle.or_else(|| account.and_then(|a| a.uid.clone())),
        capabilities: match platform {
            "codeforces" => vec!["sync".into(), "submit".into(), "statement".into()],
            "atcoder" => vec!["sync".into(), "submit".into(), "statement".into()],
            "luogu" => vec![
                "sync".into(),
                "submit".into(),
                "statement".into(),
                "human".into(),
            ],
            _ => vec![],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_luogu_record_list_result() {
        let value = json!({
            "body": {
                "currentData": {
                    "records": {
                        "result": [
                            { "id": 1, "status": 12, "problem": { "pid": "P1001" } },
                            { "id": 2, "status": 6, "problem": { "pid": "P1002" } }
                        ],
                        "count": 2,
                        "perPage": 50
                    }
                }
            }
        });
        let records = luogu_record_items(&value);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["problem"]["pid"], "P1001");
    }

    #[test]
    fn extracts_luogu_record_map_and_legacy_array() {
        let map = json!({
            "body": {
                "currentData": {
                    "records": {
                        "result": {
                            "0": { "id": 3, "problem": { "pid": "P1003" } },
                            "1": { "id": 4, "problem": { "pid": "P1004" } }
                        },
                        "count": 2
                    }
                }
            }
        });
        assert_eq!(luogu_record_items(&map).len(), 2);

        let legacy = json!({
            "body": {
                "currentData": {
                    "records": [{ "id": 5, "problem": { "pid": "P1005" } }]
                }
            }
        });
        assert_eq!(luogu_record_items(&legacy).len(), 1);
        assert!(luogu_record_items(&json!({"body": {}})).is_empty());
    }

    #[test]
    fn extracts_luogu_problem_list_result() {
        let value = json!({
            "body": {
                "currentData": {
                    "problems": {
                        "result": [
                            { "pid": "P1001", "title": "A+B Problem", "difficulty": 1 },
                            { "pid": "P1002", "title": "过河卒", "difficulty": 2 }
                        ],
                        "count": 2,
                        "perPage": 50
                    }
                }
            }
        });
        let problems = luogu_problem_items(&value);
        assert_eq!(problems.len(), 2);
        assert_eq!(problems[1]["pid"], "P1002");
    }

    #[test]
    fn extracts_lentille_records_and_user() {
        let records = json!({
            "body": {
                "instance": "main",
                "template": "record.list",
                "status": 200,
                "data": {
                    "records": {
                        "perPage": 20,
                        "count": 2,
                        "result": [
                            { "id": 9, "status": 12, "problem": { "pid": "P1746", "name": "离开中山路" } }
                        ]
                    }
                }
            }
        });
        assert_eq!(luogu_record_items(&records).len(), 1);
        assert_eq!(luogu_record_per_page(&records), Some(20));
        assert_eq!(
            luogu_record_items(&records)[0]["problem"]["name"],
            "离开中山路"
        );

        let user = json!({
            "body": {
                "data": {
                    "user": { "name": "daxingyi", "passedProblemCount": 78 },
                    "elo": []
                }
            }
        });
        let data = luogu_data(&user);
        assert_eq!(data["user"]["name"], "daxingyi");
        assert_eq!(data["user"]["passedProblemCount"], 78);
    }

    #[test]
    fn extracts_lentille_problem_list() {
        let value = json!({
            "body": {
                "data": {
                    "problems": {
                        "perPage": 50,
                        "count": 18073,
                        "result": [
                            { "pid": "P1001", "name": "A+B Problem", "difficulty": 1 }
                        ]
                    }
                }
            }
        });
        let problems = luogu_problem_items(&value);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0]["name"], "A+B Problem");
    }

    /// Shape of `GET /problem/P1111?_contentOnly=1`, as used by Luogu's own SPA.
    fn luogu_problem_fixture() -> Value {
        json!({
            "currentData": {
                "problem": {
                    "pid": "P1111",
                    "content": {
                        "name": "修复公路",
                        "background": "A 地区在地震过后，连接所有村庄的公路都造成了损坏。",
                        "description": "给出村庄数 $N$，和公路数 $M$。",
                        "formatI": "第 $1$ 行两个正整数 $N,M$。",
                        "formatO": "输出最早什么时候任意两个村庄能够通车。",
                        "hint": "$1\\leq x, y\\leq N \\le 10 ^ 3$。<br>提示结束",
                        "locale": "zh-CN"
                    },
                    "samples": [["4 4\n1 2 6\n", "5\n"]],
                    "limits": { "time": [1000], "memory": [128000] },
                    "tags": [45, 47],
                    "difficulty": 3
                }
            }
        })
    }

    #[test]
    fn composes_luogu_statement_from_markdown_fields() {
        let value = luogu_problem_fixture();
        let problem = luogu_data(&value).get("problem").cloned().unwrap();
        let markdown = luogu_statement_markdown(&problem);
        assert!(markdown.starts_with("## 题目背景"));
        for heading in ["## 题目描述", "## 输入格式", "## 输出格式", "## 说明/提示"]
        {
            assert!(markdown.contains(heading), "missing {heading}");
        }
        // Formulas are preserved verbatim for remark-math/KaTeX.
        assert!(markdown.contains("$N$"));
        assert!(markdown.contains("$1\\leq x, y\\leq N \\le 10 ^ 3$"));
        // The page chrome must never leak into the statement.
        assert!(!markdown.contains("个人中心"));
        assert!(!markdown.contains("提交答案"));
        // `<br>` becomes a real line break instead of raw HTML.
        assert!(!markdown.contains("<br>"));
    }

    #[test]
    fn skips_empty_luogu_sections() {
        let problem = json!({ "description": "只有描述", "inputFormat": "  ", "hint": null });
        let markdown = luogu_statement_markdown(&problem);
        assert_eq!(markdown, "## 题目描述\n\n只有描述");
        // Legacy top-level `inputFormat`/`outputFormat` still works.
        let legacy = json!({
            "description": "$N$",
            "inputFormat": "输入",
            "outputFormat": "输出"
        });
        let markdown = luogu_statement_markdown(&legacy);
        assert!(markdown.contains("## 输入格式\n\n输入"));
        assert!(markdown.contains("## 输出格式\n\n输出"));
    }

    #[test]
    fn converts_inline_images_to_markdown() {
        assert_eq!(
            inline_html_to_markdown(r#"<img src="https://cdn/a.png" />"#),
            "![](https://cdn/a.png)"
        );
        assert_eq!(inline_html_to_markdown("a<br/>b"), "a\nb");
    }

    #[test]
    fn converts_luogu_limits_to_ms_and_mb() {
        let value = luogu_problem_fixture();
        let problem = luogu_data(&value).get("problem").cloned().unwrap();
        assert_eq!(luogu_limits(&problem), (Some(1000), Some(125)));
        assert_eq!(luogu_limits(&json!({})), (None, None));
    }

    #[tokio::test]
    async fn rewrites_placeholder_tags_and_merges_skill_profiles() {
        let dir = std::env::temp_dir().join(format!("icpc-tags-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let db = Db::open(&dir.join("test.db")).await.expect("db");
        sqlx::query(
            "INSERT INTO problems(id, platform, native_id, title, url, normalized_rating, tags_json, skill_tags_json, samples_json)
             VALUES('luogu:P1', 'luogu', 'P1', '测试', 'https://example.com', 1500, '[\"tag-45\",\"二分\"]', '[\"tag-45\"]', '[]')",
        )
        .execute(&db.pool)
        .await
        .expect("insert problem");
        sqlx::query(
            "INSERT INTO plan_items(id, plan_id, problem_id, target_tags_json) VALUES('i1', 'p1', 'luogu:P1', '[\"tag-45\"]')",
        )
        .execute(&db.pool)
        .await
        .expect("insert item");
        for (tag, rating, attempts, solved) in [("tag-45", 1200.0, 2, 1), ("二分", 1400.0, 2, 2)]
        {
            sqlx::query(
                "INSERT INTO skill_profiles(tag, rating, attempts, solved, last_delta, updated_at)
                 VALUES(?1, ?2, ?3, ?4, 0, 'now')",
            )
            .bind(tag)
            .bind(rating)
            .bind(attempts)
            .bind(solved)
            .execute(&db.pool)
            .await
            .expect("insert skill");
        }

        let names: HashMap<i64, String> = [(45i64, "二分".to_string())].into_iter().collect();
        let changed = tidy_placeholder_tags(&db, &names).await.expect("tidy");
        assert!(
            changed >= 3,
            "problem, plan item and skill row are rewritten"
        );

        let tags: String = sqlx::query("SELECT tags_json FROM problems WHERE id='luogu:P1'")
            .fetch_one(&db.pool)
            .await
            .expect("problem")
            .get("tags_json");
        assert_eq!(tags, "[\"二分\"]");
        let plan_tags: String =
            sqlx::query("SELECT target_tags_json FROM plan_items WHERE id='i1'")
                .fetch_one(&db.pool)
                .await
                .expect("item")
                .get("target_tags_json");
        assert_eq!(plan_tags, "[\"二分\"]");

        // The Elo of both rows is merged by attempts: (1200*2 + 1400*2) / 4.
        let rows = sqlx::query("SELECT tag, rating, attempts, solved FROM skill_profiles")
            .fetch_all(&db.pool)
            .await
            .expect("skills");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get::<String, _>("tag"), "二分");
        assert_eq!(rows[0].get::<i64, _>("attempts"), 4);
        assert_eq!(rows[0].get::<i64, _>("solved"), 3);
        assert!((rows[0].get::<f64, _>("rating") - 1300.0).abs() < 0.01);

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_cjk_skill_tags() {
        assert_eq!(
            skill_tags_for(
                "luogu",
                &["并查集".to_string(), "Data Structures".to_string()]
            ),
            vec!["并查集", "data-structures"]
        );
    }

    #[test]
    fn maps_luogu_tag_ids_to_names() {
        let problem = json!({ "tags": [45, 47, 999] });
        let names: HashMap<i64, String> = [(45, "并查集".to_string()), (47, "图论".to_string())]
            .into_iter()
            .collect();
        assert_eq!(
            luogu_tags(&problem, &names),
            vec!["并查集", "图论", "tag-999"]
        );
    }

    #[test]
    fn detects_page_chrome_and_placeholder_tags() {
        let statement = "题目列表/P1111 个人中心 用户设置 提交答案 加入题单";
        assert!(luogu_statement_needs_refresh(Some(statement), "[]"));
        assert!(luogu_statement_needs_refresh(None, "[]"));
        assert!(luogu_statement_needs_refresh(
            Some("## 题目描述"),
            r#"["tag-45"]"#
        ));
        assert!(!luogu_statement_needs_refresh(
            Some("## 题目描述\n\n给出 $N$"),
            r#"["并查集"]"#
        ));
    }
}
