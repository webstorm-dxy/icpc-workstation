use crate::analysis;
use crate::automation::AutomationManager;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::llm::LlmClient;
use crate::models::{
    CaptureVerdictRequest, ManualSubmissionRequest, SettingsDto, SubmissionDto, SubmitRequest,
    SubmitResultDto,
};
use crate::platforms::{self, Account};
use crate::util::{now_iso, verdict_from_atcoder, verdict_from_cf, verdict_from_luogu};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::Row;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

struct VerdictInfo {
    native_id: String,
    verdict: String,
    language: Option<String>,
    time_ms: Option<i64>,
    memory_kb: Option<i64>,
    submitted_at: String,
    raw: Value,
}

fn epoch_to_iso(secs: i64) -> String {
    DateTime::<Utc>::from_timestamp(secs, 0)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(now_iso)
}

async fn account_for(db: &Db, platform: &str) -> AppResult<Account> {
    platforms::load_account(db, platform)
        .await?
        .ok_or_else(|| AppError::new("E_PLATFORM_HANDLE", "尚未配置该平台账号"))
}

fn cooldown_secs(platform: &str) -> i64 {
    match platform {
        "atcoder" => 300,
        _ => 60,
    }
}

async fn check_cooldown(db: &Db, platform: &str) -> AppResult<()> {
    let row = sqlx::query(
        "SELECT submitted_at FROM submissions WHERE platform = ?1 ORDER BY submitted_at DESC LIMIT 1",
    )
    .bind(platform)
    .fetch_optional(&db.pool)
    .await?;
    if let Some(row) = row {
        if let Ok(dt) = DateTime::parse_from_rfc3339(row.get::<String, _>("submitted_at").as_str())
        {
            let age = Utc::now()
                .signed_duration_since(dt.with_timezone(&Utc))
                .num_seconds();
            let min = cooldown_secs(platform);
            if age >= 0 && age < min {
                return Err(AppError::new(
                    "E_SUBMIT_COOLDOWN",
                    format!(
                        "为降低风控风险，{} 提交冷却中，还需等待 {} 秒",
                        platforms::platform_name(platform),
                        min - age
                    ),
                ));
            }
        }
    }
    Ok(())
}

pub async fn submit(
    app: &AppHandle,
    db: &Db,
    automation: &AutomationManager,
    llm: &LlmClient,
    req: SubmitRequest,
) -> AppResult<SubmitResultDto> {
    let problem = sqlx::query(
        "SELECT id, platform, native_id, url, title, time_limit_ms FROM problems WHERE id = ?1",
    )
    .bind(&req.problem_id)
    .fetch_optional(&db.pool)
    .await?
    .ok_or_else(|| AppError::not_found("题目"))?;
    let platform: String = problem.get("platform");
    let native_id: String = problem.get("native_id");
    let problem_url: String = problem.get("url");
    let title: String = problem.get("title");
    let account = account_for(db, &platform).await?;
    check_cooldown(db, &platform).await?;
    let code_hash = crate::util::sha256_hex(&req.code);
    let duplicate = sqlx::query(
        "SELECT id FROM submissions WHERE platform = ?1 AND code_sha256 = ?2 AND submitted_at > ?3 LIMIT 1",
    )
    .bind(&platform)
    .bind(&code_hash)
    .bind(
        DateTime::<Utc>::from_timestamp(Utc::now().timestamp() - 600, 0)
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_else(now_iso),
    )
    .fetch_optional(&db.pool)
    .await?;
    if duplicate.is_some() {
        return Err(AppError::new(
            "E_DUPLICATE_SUBMISSION",
            "同样的代码刚刚已经提交过，请避免重复提交",
        ));
    }
    if (platform == "codeforces" || platform == "atcoder") && !account.logged_in {
        return Ok(SubmitResultDto {
            needs_human: true,
            message: format!(
                "{} 的登录状态尚未确认，请先在自动化面板登录后再提交",
                platforms::platform_name(&platform)
            ),
            submission: None,
            automation_status: "needs_login".into(),
        });
    }
    let settings = db
        .get_json::<SettingsDto>("settings")
        .await
        .unwrap_or_default();
    let auto = req
        .auto_submit
        .unwrap_or(settings.auto_submit && !settings.require_submit_confirm);

    let submit_url = submit_page_url(&platform, &native_id, &problem_url);
    automation
        .open(app, &platform, &submit_url, Some(platforms::DEFAULT_BOUNDS))
        .await?;
    automation.wait_ready(&platform, 30_000).await?;
    automation
        .set_status(app, &platform, "running", format!("正在填写 {title}…"))
        .await;

    if let Some(message) = check_page(&platform, automation).await? {
        automation
            .needs_human(app, &platform, &message.0, &message.1)
            .await;
        return Ok(SubmitResultDto {
            needs_human: true,
            message: message.0,
            submission: None,
            automation_status: "needs_human".into(),
        });
    }

    if platform == "luogu" {
        // Luogu's submit form lives in a tab/modal opened by the button.
        let _ = automation
            .eval_json(&platform, click_submit_tab(), 5_000)
            .await;
        tokio::time::sleep(Duration::from_millis(1_400)).await;
    }

    let language = req.language.clone().unwrap_or_default();
    let fill = fill_script(&platform, &req.code, &native_id, &language);
    let fill_result = automation.eval_json(&platform, &fill, 20_000).await?;
    if fill_result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let detail = fill_result
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("页面结构可能已变化");
        automation
            .needs_human(
                app,
                &platform,
                format!("无法自动填写代码：{detail}"),
                "请在右侧页面中粘贴代码并手动提交。应用会在你点击“保存结果”后尝试读取判定。",
            )
            .await;
        return Ok(SubmitResultDto {
            needs_human: true,
            message: format!("无法自动填写代码：{detail}"),
            submission: None,
            automation_status: "manual_fill".into(),
        });
    }
    let used_language = fill_result
        .get("language")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| Some(language.clone()).filter(|s| !s.is_empty()));

    if !auto {
        automation
            .needs_human(
                app,
                &platform,
                "代码已填入，请确认后提交",
                "检查语言与代码后在右侧页面点击提交；提交完成后回到应用点击“我已提交，读取结果”。",
            )
            .await;
        return Ok(SubmitResultDto {
            needs_human: true,
            message: "代码已填入，请在页面中确认并提交".into(),
            submission: None,
            automation_status: "waiting_for_manual_submit".into(),
        });
    }

    let since = Utc::now().timestamp() - 3;
    automation
        .eval_json(&platform, click_submit_script(&platform), 10_000)
        .await?;
    automation
        .set_status(app, &platform, "judging", "已提交，正在等待判定…")
        .await;
    tokio::time::sleep(Duration::from_millis(2_000)).await;
    if let Some((message, hint)) = check_page(&platform, automation).await? {
        automation
            .needs_human(app, &platform, &message, &hint)
            .await;
        return Ok(SubmitResultDto {
            needs_human: true,
            message,
            submission: None,
            automation_status: "needs_human".into(),
        });
    }
    match poll_verdict(app, db, automation, &platform, &native_id, since, 150).await {
        Ok(info) => {
            let submission_id = insert_captured(
                db,
                &platform,
                &req.problem_id,
                &req.code,
                used_language.as_deref(),
                info,
                "automation",
            )
            .await?;
            spawn_analysis(app, db, llm, &submission_id);
            let dto = submission_dto(db, &submission_id).await?;
            let _ = app.emit("submission://updated", json!({ "id": submission_id }));
            automation
                .set_status(app, &platform, "done", "已读取判定结果")
                .await;
            Ok(SubmitResultDto {
                needs_human: false,
                message: format!("提交完成：{}", dto.verdict),
                submission: Some(dto),
                automation_status: "done".into(),
            })
        }
        Err(err) => {
            automation
                .needs_human(
                    app,
                    &platform,
                    "已提交，但暂时没有读到判定",
                    "请等待评测完成后点击“读取结果”；如果页面显示需要验证，请先完成验证。",
                )
                .await;
            Ok(SubmitResultDto {
                needs_human: true,
                message: format!("{}：请稍后点击“读取结果”", err.message),
                submission: None,
                automation_status: "verdict_pending".into(),
            })
        }
    }
}

pub async fn capture_verdict(
    app: &AppHandle,
    db: &Db,
    automation: &AutomationManager,
    llm: &LlmClient,
    req: CaptureVerdictRequest,
) -> AppResult<SubmitResultDto> {
    let problem = sqlx::query("SELECT platform, native_id FROM problems WHERE id = ?1")
        .bind(&req.problem_id)
        .fetch_optional(&db.pool)
        .await?
        .ok_or_else(|| AppError::not_found("题目"))?;
    let platform: String = problem.get("platform");
    let native_id: String = problem.get("native_id");
    automation
        .set_status(app, &platform, "judging", "正在读取判定结果…")
        .await;
    let since = req
        .since_epoch
        .unwrap_or_else(|| Utc::now().timestamp() - 900);
    let info = poll_verdict(app, db, automation, &platform, &native_id, since, 120).await?;
    let submission_id = insert_captured(
        db,
        &platform,
        &req.problem_id,
        &req.code,
        req.language.as_deref(),
        info,
        "automation",
    )
    .await?;
    spawn_analysis(app, db, llm, &submission_id);
    let dto = submission_dto(db, &submission_id).await?;
    let _ = app.emit("submission://updated", json!({ "id": submission_id }));
    Ok(SubmitResultDto {
        needs_human: false,
        message: format!("已读取判定：{}", dto.verdict),
        submission: Some(dto),
        automation_status: "done".into(),
    })
}

pub async fn record_manual(
    app: &AppHandle,
    db: &Db,
    llm: &LlmClient,
    req: ManualSubmissionRequest,
) -> AppResult<SubmissionDto> {
    let problem = sqlx::query("SELECT platform FROM problems WHERE id = ?1")
        .bind(&req.problem_id)
        .fetch_optional(&db.pool)
        .await?
        .ok_or_else(|| AppError::not_found("题目"))?;
    let platform: String = problem.get("platform");
    let native = format!("manual-{}", uuid::Uuid::now_v7());
    let info = VerdictInfo {
        native_id: native.clone(),
        verdict: crate::util::norm_verdict(&req.verdict),
        language: req.language.clone(),
        time_ms: req.time_ms,
        memory_kb: req.memory_kb,
        submitted_at: now_iso(),
        raw: json!({ "manual": true }),
    };
    let submission_id = insert_captured(
        db,
        &platform,
        &req.problem_id,
        &req.code,
        req.language.as_deref(),
        info,
        "manual",
    )
    .await?;
    spawn_analysis(app, db, llm, &submission_id);
    let dto = submission_dto(db, &submission_id).await?;
    let _ = app.emit("submission://updated", json!({ "id": submission_id }));
    Ok(dto)
}

fn spawn_analysis(app: &AppHandle, db: &Db, llm: &LlmClient, submission_id: &str) {
    let app = app.clone();
    let db = db.clone();
    let llm = llm.clone();
    let id = submission_id.to_string();
    tokio::spawn(async move {
        let _ = analysis::analyze_submission(&app, &db, &llm, &id, false).await;
    });
}

async fn insert_captured(
    db: &Db,
    platform: &str,
    problem_id: &str,
    code: &str,
    language: Option<&str>,
    info: VerdictInfo,
    captured_via: &str,
) -> AppResult<String> {
    let submission_id = format!("{}:s{}", platform, info.native_id);
    let code_hash = crate::util::sha256_hex(code);
    sqlx::query(
        "INSERT INTO submissions(id, platform, native_id, problem_id, verdict, language, code_sha256, time_ms, memory_kb, submitted_at, captured_via, raw_json, analysis_status)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'pending')
         ON CONFLICT(platform, native_id) DO UPDATE SET
           verdict = excluded.verdict, language = COALESCE(excluded.language, submissions.language),
           code_sha256 = excluded.code_sha256, time_ms = excluded.time_ms, memory_kb = excluded.memory_kb,
           captured_via = excluded.captured_via, analysis_status = 'pending'",
    )
    .bind(&submission_id)
    .bind(platform)
    .bind(&info.native_id)
    .bind(problem_id)
    .bind(&info.verdict)
    .bind(info.language.clone().or_else(|| language.map(|s| s.to_string())))
    .bind(&code_hash)
    .bind(info.time_ms)
    .bind(info.memory_kb)
    .bind(&info.submitted_at)
    .bind(captured_via)
    .bind(info.raw.to_string())
    .execute(&db.pool)
    .await?;
    sqlx::query(
        "INSERT INTO submission_code(submission_id, code_text) VALUES(?1, ?2)
         ON CONFLICT(submission_id) DO UPDATE SET code_text = excluded.code_text",
    )
    .bind(&submission_id)
    .bind(code)
    .execute(&db.pool)
    .await?;
    Ok(submission_id)
}

pub async fn submission_dto(db: &Db, submission_id: &str) -> AppResult<SubmissionDto> {
    let row = sqlx::query(
        "SELECT s.id, s.platform, s.problem_id, s.verdict, s.language, s.time_ms, s.memory_kb,
                s.submitted_at, s.analysis_status, p.title
         FROM submissions s JOIN problems p ON p.id = s.problem_id WHERE s.id = ?1",
    )
    .bind(submission_id)
    .fetch_optional(&db.pool)
    .await?
    .ok_or_else(|| AppError::not_found("提交记录"))?;
    let analysis = analysis::load_analysis(db, submission_id).await;
    Ok(SubmissionDto {
        id: row.get("id"),
        platform: row.get("platform"),
        problem_id: row.get("problem_id"),
        problem_title: row.get("title"),
        verdict: row.get("verdict"),
        language: row.get("language"),
        time_ms: row.get("time_ms"),
        memory_kb: row.get("memory_kb"),
        submitted_at: row.get("submitted_at"),
        analysis_status: row.get("analysis_status"),
        needs_human: false,
        message: None,
        analysis,
    })
}

fn submit_page_url(platform: &str, native_id: &str, problem_url: &str) -> String {
    match platform {
        "codeforces" => {
            let split = native_id
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(native_id.len());
            let contest = &native_id[..split];
            if contest.is_empty() {
                problem_url.to_string()
            } else {
                format!("https://codeforces.com/contest/{contest}/submit")
            }
        }
        "atcoder" => {
            let contest = native_id.split('_').next().unwrap_or(native_id);
            format!("https://atcoder.jp/contests/{contest}/submit?taskScreenName={native_id}")
        }
        _ => problem_url.to_string(),
    }
}

async fn check_page(
    platform: &str,
    automation: &AutomationManager,
) -> AppResult<Option<(String, String)>> {
    let probe = automation
        .eval_json(platform, platforms::challenge_probe(), 6_000)
        .await?;
    if probe.get("challenge").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(Some((
            format!("{} 需要完成人机验证", platforms::platform_name(platform)),
            "请在右侧页面完成验证，然后点击“继续”。".into(),
        )));
    }
    if probe.get("login").and_then(|v| v.as_bool()) == Some(true) {
        return Ok(Some((
            format!("{} 登录已失效", platforms::platform_name(platform)),
            "请在右侧页面完成登录，然后重新提交。".into(),
        )));
    }
    Ok(None)
}

fn payload(code: &str, native_id: &str, language: &str) -> String {
    json!({ "code": code, "native": native_id, "language": language }).to_string()
}

fn fill_script(platform: &str, code: &str, native_id: &str, language: &str) -> String {
    let p = payload(code, native_id, language);
    let body = match platform {
        "codeforces" => CODEFORCES_FILL,
        "atcoder" => ATCODER_FILL,
        _ => LUOGU_FILL,
    };
    body.replace("__PAYLOAD__", &p)
}

fn click_submit_tab() -> &'static str {
    r#"(() => {
  const btn = Array.from(document.querySelectorAll('button, a')).find(b => /^提交$|提交代码|Submit/.test((b.innerText || '').trim()));
  if (btn) { btn.click(); return JSON.stringify({ ok: true }); }
  return JSON.stringify({ ok: false, error: '没有找到提交按钮' });
})()"#
}

fn click_submit_script(platform: &str) -> &'static str {
    match platform {
        "codeforces" => {
            r#"(() => {
  const buttons = Array.from(document.querySelectorAll('input[type="submit"], button[type="submit"], button, input[type="button"]'));
  const btn = buttons.find(b => /submit/i.test(b.value || '') || /submit/i.test((b.innerText || '').trim()));
  if (!btn) return JSON.stringify({ ok: false, error: '没有找到提交按钮' });
  btn.click();
  return JSON.stringify({ ok: true });
})()"#
        }
        "atcoder" => {
            r#"(() => {
  const buttons = Array.from(document.querySelectorAll('button[type="submit"], input[type="submit"], button'));
  const btn = buttons.find(b => /submit|提出/i.test((b.innerText || '') + ' ' + (b.value || '')));
  if (!btn) return JSON.stringify({ ok: false, error: '没有找到提交按钮' });
  btn.click();
  return JSON.stringify({ ok: true });
})()"#
        }
        _ => {
            r#"(() => {
  const buttons = Array.from(document.querySelectorAll('button, input[type="submit"]'));
  const btn = buttons.find(b => /提交|submit/i.test((b.innerText || '') + ' ' + (b.value || '')));
  if (!btn) return JSON.stringify({ ok: false, error: '没有找到提交按钮' });
  btn.click();
  return JSON.stringify({ ok: true });
})()"#
        }
    }
}

const CODEFORCES_FILL: &str = r#"(() => {
  try {
    const p = __PAYLOAD__;
    const setVal = (el, v) => {
      const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
      setter.call(el, v);
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    };
    const problemSelect = document.querySelector('select[name="submittedProblemIndex"]');
    if (problemSelect) {
      const opt = Array.from(problemSelect.options).find(o => o.value === p.native.slice(-1) || o.value === p.native);
      if (opt) {
        problemSelect.value = opt.value;
        problemSelect.dispatchEvent(new Event('change', { bubbles: true }));
      }
    }
    const langSelect = document.querySelector('select[name="programTypeId"]');
    let chosen = '';
    if (langSelect) {
      const opts = Array.from(langSelect.options);
      const opt = opts.find(o => /GNU G\+\+20/.test(o.text)) || opts.find(o => /GNU G\+\+17/.test(o.text)) || opts.find(o => /GNU G\+\+/.test(o.text));
      if (opt) {
        langSelect.value = opt.value;
        chosen = opt.text.trim();
        langSelect.dispatchEvent(new Event('change', { bubbles: true }));
      }
    }
    const cm = document.querySelector('.CodeMirror');
    let editor = 'none';
    if (cm && cm.CodeMirror) {
      cm.CodeMirror.setValue(p.code);
      editor = 'codemirror';
    }
    const ta = document.querySelector('textarea[name="source"], textarea[name="sourceCode"], #source');
    if (ta) {
      setVal(ta, p.code);
      editor = editor === 'none' ? 'textarea' : editor;
    }
    if (editor === 'none') return JSON.stringify({ ok: false, error: '没有找到 Codeforces 代码编辑器' });
    return JSON.stringify({ ok: true, language: chosen || p.language, editor });
  } catch (e) {
    return JSON.stringify({ ok: false, error: String(e) });
  }
})()"#;

const ATCODER_FILL: &str = r#"(() => {
  try {
    const p = __PAYLOAD__;
    const setVal = (el, v) => {
      const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
      setter.call(el, v);
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    };
    const taskSelect = document.querySelector('select[name="data.TaskScreenName"]');
    if (taskSelect) {
      const opt = Array.from(taskSelect.options).find(o => o.value === p.native);
      if (opt) {
        taskSelect.value = opt.value;
        taskSelect.dispatchEvent(new Event('change', { bubbles: true }));
      }
    }
    const langSelect = document.querySelector('select[name="data.LanguageId"]');
    let chosen = '';
    if (langSelect) {
      const opts = Array.from(langSelect.options);
      const opt = opts.find(o => /C\+\+ 20/.test(o.text)) || opts.find(o => /C\+\+ 17/.test(o.text)) || opts.find(o => /C\+\+/.test(o.text));
      if (opt) {
        langSelect.value = opt.value;
        chosen = opt.text.trim();
        langSelect.dispatchEvent(new Event('change', { bubbles: true }));
      }
    }
    const ta = document.querySelector('textarea[name="sourceCode"], textarea#sourceCode, textarea');
    if (!ta) return JSON.stringify({ ok: false, error: '没有找到 AtCoder 代码输入框' });
    setVal(ta, p.code);
    return JSON.stringify({ ok: true, language: chosen || p.language, editor: 'textarea' });
  } catch (e) {
    return JSON.stringify({ ok: false, error: String(e) });
  }
})()"#;

const LUOGU_FILL: &str = r#"(() => {
  try {
    const p = __PAYLOAD__;
    const setVal = (el, v) => {
      const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
      setter.call(el, v);
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
    };
    const selects = Array.from(document.querySelectorAll('select'));
    const langSelect = selects.find(s => Array.from(s.options).some(o => /C\+\+/.test(o.text)));
    let chosen = '';
    if (langSelect) {
      const opt = Array.from(langSelect.options).find(o => /C\+\+17/.test(o.text)) || Array.from(langSelect.options).find(o => /C\+\+20/.test(o.text)) || Array.from(langSelect.options).find(o => /C\+\+/.test(o.text));
      if (opt) {
        langSelect.value = opt.value;
        chosen = opt.text.trim();
        langSelect.dispatchEvent(new Event('change', { bubbles: true }));
      }
    }
    const cm = document.querySelector('.CodeMirror');
    let editor = 'none';
    if (cm && cm.CodeMirror) { cm.CodeMirror.setValue(p.code); editor = 'codemirror'; }
    if (window.monaco && window.monaco.editor) {
      const models = window.monaco.editor.getModels();
      if (models && models.length) { models[0].setValue(p.code); editor = 'monaco'; }
    }
    if (window.ace) {
      const aceNode = document.querySelector('.ace_editor');
      if (aceNode) { window.ace.edit(aceNode).setValue(p.code, -1); editor = 'ace'; }
    }
    const ta = document.querySelector('textarea');
    if (ta) { setVal(ta, p.code); editor = editor === 'none' ? 'textarea' : editor; }
    if (editor === 'none') return JSON.stringify({ ok: false, error: '没有找到洛谷代码编辑器，请在页面中手动粘贴' });
    return JSON.stringify({ ok: true, language: chosen || p.language, editor });
  } catch (e) {
    return JSON.stringify({ ok: false, error: String(e) });
  }
})()"#;

async fn poll_verdict(
    app: &AppHandle,
    db: &Db,
    automation: &AutomationManager,
    platform: &str,
    native_id: &str,
    since_epoch: i64,
    timeout_secs: u64,
) -> AppResult<VerdictInfo> {
    let started = tokio::time::Instant::now();
    let deadline = started + Duration::from_secs(timeout_secs);
    loop {
        let result = match platform {
            "codeforces" => poll_codeforces(db, native_id, since_epoch).await,
            "atcoder" => poll_atcoder(db, native_id, since_epoch).await,
            "luogu" => poll_luogu(app, db, automation, native_id, since_epoch).await,
            _ => Err(AppError::invalid("未知平台")),
        };
        match result {
            Ok(Some(info)) => return Ok(info),
            Ok(None) => {}
            Err(err) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(err);
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(
                AppError::new("E_VERDICT_TIMEOUT", "等待判定超时，请稍后点击“读取结果”")
                    .retryable(),
            );
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn poll_codeforces(
    db: &Db,
    native_id: &str,
    since_epoch: i64,
) -> AppResult<Option<VerdictInfo>> {
    let account = account_for(db, "codeforces").await?;
    let handle = account
        .handle
        .ok_or_else(|| AppError::new("E_PLATFORM_HANDLE", "请先填写 Codeforces 用户名"))?;
    let client = platforms::http_client().await?;
    let value = platforms::cf_api(
        &client,
        &format!("user.status?handle={handle}&from=1&count=20"),
    )
    .await?;
    let Some(items) = value.as_array() else {
        return Ok(None);
    };
    for item in items {
        let problem = match item.get("problem") {
            Some(p) => p,
            None => continue,
        };
        let contest = problem
            .get("contestId")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let index = problem
            .get("index")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if format!("{contest}{index}") != native_id {
            continue;
        }
        let created = item
            .get("creationTimeSeconds")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        if created < since_epoch {
            continue;
        }
        let verdict_raw = item
            .get("verdict")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN");
        let verdict = verdict_from_cf(verdict_raw);
        if verdict == "JUDGING" {
            return Ok(None);
        }
        let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
        return Ok(Some(VerdictInfo {
            native_id: id.to_string(),
            verdict,
            language: item
                .get("programmingLanguage")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            time_ms: item.get("timeConsumedMillis").and_then(|v| v.as_i64()),
            memory_kb: item
                .get("memoryConsumedBytes")
                .and_then(|v| v.as_i64())
                .map(|b| b / 1024),
            submitted_at: epoch_to_iso(created),
            raw: item.clone(),
        }));
    }
    Ok(None)
}

async fn poll_atcoder(
    db: &Db,
    native_id: &str,
    since_epoch: i64,
) -> AppResult<Option<VerdictInfo>> {
    let account = account_for(db, "atcoder").await?;
    let handle = account
        .handle
        .ok_or_else(|| AppError::new("E_PLATFORM_HANDLE", "请先填写 AtCoder 用户名"))?;
    let client = platforms::http_client().await?;
    let url = format!(
        "https://kenkoooo.com/atcoder/atcoder-api/v3/user/submissions?user={handle}&from_second={}",
        (since_epoch - 5).max(0)
    );
    let resp = client.get(&url).send().await.map_err(AppError::network)?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let value: Value = resp.json().await.unwrap_or(json!([]));
    let Some(items) = value.as_array() else {
        return Ok(None);
    };
    let mut best: Option<&Value> = None;
    for item in items {
        if item.get("problem_id").and_then(|v| v.as_str()) != Some(native_id) {
            continue;
        }
        let epoch = item
            .get("epoch_second")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        if epoch < since_epoch - 5 {
            continue;
        }
        if best
            .map(|b| b.get("epoch_second").and_then(|v| v.as_i64()).unwrap_or(0) < epoch)
            .unwrap_or(true)
        {
            best = Some(item);
        }
    }
    let Some(item) = best else {
        return Ok(None);
    };
    let verdict = verdict_from_atcoder(
        item.get("result")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN"),
    );
    if verdict == "JUDGING" {
        return Ok(None);
    }
    let epoch = item
        .get("epoch_second")
        .and_then(|v| v.as_i64())
        .unwrap_or_default();
    let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
    Ok(Some(VerdictInfo {
        native_id: id.to_string(),
        verdict,
        language: item
            .get("language")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        time_ms: item.get("execution_time").and_then(|v| v.as_i64()),
        memory_kb: None,
        submitted_at: epoch_to_iso(epoch),
        raw: item.clone(),
    }))
}

async fn poll_luogu(
    app: &AppHandle,
    db: &Db,
    automation: &AutomationManager,
    native_id: &str,
    since_epoch: i64,
) -> AppResult<Option<VerdictInfo>> {
    let _ = app;
    let account = account_for(db, "luogu").await?;
    let uid = account
        .uid
        .ok_or_else(|| AppError::new("E_PLATFORM_HANDLE", "请先填写洛谷 UID"))?;
    let value = automation
        .fetch_json_in_page(
            "luogu",
            &format!("/record/list?user_id={uid}&page=1&_contentOnly=1"),
            25_000,
        )
        .await?;
    let records = value
        .get("body")
        .and_then(|b| b.get("currentData"))
        .and_then(|d| d.get("records"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for record in records {
        let pid = record
            .get("problem")
            .and_then(|p| p.get("pid"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if pid != native_id {
            continue;
        }
        let submit_time = record
            .get("submitTime")
            .and_then(|v| v.as_i64())
            .unwrap_or_default();
        let epoch = if submit_time > 1_000_000_000_000 {
            submit_time / 1000
        } else {
            submit_time
        };
        if epoch < since_epoch - 5 {
            continue;
        }
        let status = record.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
        let verdict = verdict_from_luogu(status);
        if verdict == "JUDGING" || verdict == "UNKNOWN" {
            return Ok(None);
        }
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
            .unwrap_or_else(|| format!("{pid}-{epoch}"));
        return Ok(Some(VerdictInfo {
            native_id: id,
            verdict,
            language: record
                .get("language")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            time_ms: record.get("time").and_then(|v| v.as_i64()),
            memory_kb: record.get("memory").and_then(|v| v.as_i64()),
            submitted_at: epoch_to_iso(epoch),
            raw: record.clone(),
        }));
    }
    Ok(None)
}
