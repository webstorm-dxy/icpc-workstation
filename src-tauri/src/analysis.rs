use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::llm::LlmClient;
use crate::models::{AnalysisDto, SettingsDto, TagStatDto};
use crate::provider;
use crate::util::{is_ac, now_iso, parse_json_array, truncate_chars};
use serde_json::{Value, json};
use sqlx::Row;
use tauri::{AppHandle, Emitter};

const PROMPT_VERSION: &str = "analyst/v1";

const ANALYST_SYSTEM: &str = r#"你是 ICPC 竞赛教练，负责在选手提交代码后做严格、具体、可执行的代码复盘。
规则：
1. 只根据给出的题面、代码、提交结果与本地样例证据判断，不要编造题目没有的信息。
2. 失败时定位到具体错误（越界、溢出、边界、复杂度、I/O 格式等），不要说空泛的“多练习”。
3. 即使通过，也要检查复杂度是否贴近限制、是否存在隐患，并给出下一难度建议。
4. 只输出一个 JSON 对象，不要 markdown 代码块，不要额外解释。
5. 所有文本用简体中文。"#;

fn analysis_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["summary", "failure_category", "root_cause", "bug_patterns", "evidence", "complexity", "fix_steps", "next_practice_tags", "learning_points", "confidence"],
        "properties": {
            "summary": {"type": "string", "description": "一两句话结论"},
            "failure_category": {"type": "string", "enum": ["algorithm", "implementation", "complexity", "edge_case", "io", "overflow", "recursion", "unknown", "none"]},
            "root_cause": {"type": "string", "description": "具体根因；AC 时写通过原因与可改进点"},
            "bug_patterns": {"type": "array", "items": {"type": "string"}, "description": "如 off-by-one / 未开 long long / 多测未清空"},
            "evidence": {"type": "array", "items": {"type": "string"}, "description": "来自 verdict、样例 diff、代码行的证据"},
            "complexity": {
                "type": "object",
                "required": ["time", "space", "expected", "verdict"],
                "properties": {
                    "time": {"type": "string"},
                    "space": {"type": "string"},
                    "expected": {"type": "string"},
                    "verdict": {"type": "string", "enum": ["ok", "too_slow", "too_much", "unknown"]}
                }
            },
            "fix_steps": {"type": "array", "items": {"type": "string"}},
            "next_practice_tags": {"type": "array", "items": {"type": "string"}},
            "learning_points": {"type": "array", "items": {"type": "string"}},
            "confidence": {"type": "number", "minimum": 0, "maximum": 1}
        }
    })
}

pub async fn load_analysis(db: &Db, submission_id: &str) -> Option<AnalysisDto> {
    let row = sqlx::query(
        "SELECT submission_id, provider_id, model_id, payload_json, created_at FROM analyses WHERE submission_id = ?1",
    )
    .bind(submission_id)
    .fetch_optional(&db.pool)
    .await
    .ok()
    .flatten()?;
    let payload: Value = serde_json::from_str(row.get::<&str, _>("payload_json")).ok()?;
    Some(to_dto(
        &payload,
        row.get("submission_id"),
        row.get("provider_id"),
        row.get("model_id"),
        row.get("created_at"),
    ))
}

fn to_dto(
    payload: &Value,
    submission_id: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    created_at: String,
) -> AnalysisDto {
    AnalysisDto {
        submission_id,
        summary: payload
            .get("summary")
            .and_then(|v| v.as_str())
            .unwrap_or("暂无分析")
            .to_string(),
        failure_category: payload
            .get("failure_category")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        root_cause: payload
            .get("root_cause")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        bug_patterns: string_list(payload.get("bug_patterns")),
        evidence: string_list(payload.get("evidence")),
        complexity: payload.get("complexity").cloned().unwrap_or_else(
            || json!({"time":"未知","space":"未知","expected":"未知","verdict":"unknown"}),
        ),
        fix_steps: string_list(payload.get("fix_steps")),
        next_practice_tags: string_list(payload.get("next_practice_tags")),
        learning_points: string_list(payload.get("learning_points")),
        confidence: payload
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        created_at,
        provider_id,
        model_id,
    }
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

pub async fn analyze_submission(
    app: &AppHandle,
    db: &Db,
    llm: &LlmClient,
    submission_id: &str,
    force: bool,
) -> AppResult<Option<AnalysisDto>> {
    if !force {
        if let Some(existing) = load_analysis(db, submission_id).await {
            return Ok(Some(existing));
        }
    }
    let row = sqlx::query(
        "SELECT s.id, s.platform, s.problem_id, s.verdict, s.language, s.time_ms, s.memory_kb,
                p.title, p.rating, p.normalized_rating, p.skill_tags_json, p.statement_md,
                c.code_text
         FROM submissions s
         JOIN problems p ON p.id = s.problem_id
         LEFT JOIN submission_code c ON c.submission_id = s.id
         WHERE s.id = ?1",
    )
    .bind(submission_id)
    .fetch_optional(&db.pool)
    .await?
    .ok_or_else(|| AppError::not_found("提交记录"))?;
    let code: Option<String> = row.get("code_text");
    let Some(code) = code else {
        let _ = sqlx::query("UPDATE submissions SET analysis_status = 'skipped' WHERE id = ?1")
            .bind(submission_id)
            .execute(&db.pool)
            .await;
        return Ok(None);
    };
    let verdict: String = row.get("verdict");
    let problem_id: String = row.get("problem_id");
    let title: String = row.get("title");
    let platform: String = row.get("platform");
    let tags = parse_json_array(Some(row.get::<&str, _>("skill_tags_json").to_string()));
    let normalized_rating = row
        .get::<Option<i64>, _>("normalized_rating")
        .unwrap_or(1500);
    let statement = row
        .get::<Option<String>, _>("statement_md")
        .unwrap_or_default();
    let time_ms = row.get::<Option<i64>, _>("time_ms");
    let memory_kb = row.get::<Option<i64>, _>("memory_kb");
    let settings = db
        .get_json::<SettingsDto>("settings")
        .await
        .unwrap_or_default();

    let _ = sqlx::query("UPDATE submissions SET analysis_status = 'running' WHERE id = ?1")
        .bind(submission_id)
        .execute(&db.pool)
        .await;

    let evidence = collect_evidence(db, &problem_id, &verdict, time_ms).await;
    let mut provider_id = None;
    let mut model_id = None;
    let payload = if !settings.allow_code_upload {
        deterministic_analysis(&title, &verdict, &tags, &evidence, time_ms)
    } else {
        match provider::resolve_role(db, "analyze", &provider::session_for("analyze")).await {
            Ok(resolved) => {
                provider_id = Some(resolved.provider_id.clone());
                model_id = Some(resolved.model.clone());
                let user = build_prompt(
                    &title,
                    &platform,
                    &verdict,
                    &tags,
                    normalized_rating,
                    &statement,
                    &code,
                    &evidence,
                    time_ms,
                    memory_kb,
                );
                match llm
                    .complete_json(
                        &resolved.config,
                        &resolved.model,
                        ANALYST_SYSTEM,
                        &user,
                        analysis_schema(),
                        2_500,
                    )
                    .await
                {
                    Ok(value) => value,
                    Err(err) => {
                        let mut fallback =
                            deterministic_analysis(&title, &verdict, &tags, &evidence, time_ms);
                        fallback["summary"] = json!(format!(
                            "{}（AI 分析不可用：{}）",
                            fallback["summary"].as_str().unwrap_or(""),
                            err.message
                        ));
                        fallback
                    }
                }
            }
            Err(err) => {
                let mut fallback =
                    deterministic_analysis(&title, &verdict, &tags, &evidence, time_ms);
                fallback["summary"] = json!(format!(
                    "{}（未配置分析模型：{}）",
                    fallback["summary"].as_str().unwrap_or(""),
                    err.message
                ));
                fallback
            }
        }
    };
    let normalized_payload = normalize_payload(payload);

    sqlx::query(
        "INSERT INTO analyses(id, submission_id, provider_id, model_id, prompt_version, payload_json, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(submission_id) DO UPDATE SET
           provider_id = excluded.provider_id, model_id = excluded.model_id,
           prompt_version = excluded.prompt_version, payload_json = excluded.payload_json,
           created_at = excluded.created_at",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(submission_id)
    .bind(&provider_id)
    .bind(&model_id)
    .bind(PROMPT_VERSION)
    .bind(normalized_payload.to_string())
    .bind(now_iso())
    .execute(&db.pool)
    .await?;
    sqlx::query("UPDATE submissions SET analysis_status = 'done' WHERE id = ?1")
        .bind(submission_id)
        .execute(&db.pool)
        .await?;

    update_skill_profile(db, &tags, normalized_rating, &verdict).await?;
    if is_ac(&verdict) {
        let _ = sqlx::query(
            "UPDATE plan_items SET status = 'done', completed_at = ?1
             WHERE problem_id = ?2 AND status != 'done'
               AND plan_id IN (SELECT id FROM training_plans WHERE status = 'active')",
        )
        .bind(now_iso())
        .bind(&problem_id)
        .execute(&db.pool)
        .await;
    }
    let _ = app.emit(
        "analysis://updated",
        json!({ "submissionId": submission_id, "status": "done" }),
    );
    let dto = load_analysis(db, submission_id).await;
    if settings.auto_refresh_plan {
        crate::plan::spawn_refresh(
            app.clone(),
            db.clone(),
            llm.clone(),
            Some(submission_id.to_string()),
            Some(problem_id),
        );
    }
    Ok(dto)
}

#[allow(clippy::too_many_arguments)]
fn build_prompt(
    title: &str,
    platform: &str,
    verdict: &str,
    tags: &[String],
    rating: i64,
    statement: &str,
    code: &str,
    evidence: &[String],
    time_ms: Option<i64>,
    memory_kb: Option<i64>,
) -> String {
    format!(
        r#"题目：{title}
平台：{platform}
标签：{tags}
规范难度：{rating}
提交结果：{verdict}
用时/内存：{time} / {memory}
本地证据：
{evidence}

题面（可能截断）：
{statement}

代码：
```cpp
{code}
```

输出 JSON 字段：
- summary: 一两句话结论
- failure_category: algorithm/implementation/complexity/edge_case/io/overflow/recursion/unknown/none
- root_cause: 具体根因；AC 时写通过原因与可改进点
- bug_patterns: 字符串数组，具体 bug 模式
- evidence: 字符串数组，引用代码行、样例 diff 或 verdict
- complexity: {{time, space, expected, verdict(ok/too_slow/too_much/unknown)}}
- fix_steps: 字符串数组，按顺序给出最小修复步骤
- next_practice_tags: 字符串数组
- learning_points: 字符串数组
- confidence: 0 到 1"#,
        tags = tags.join(", "),
        time = time_ms
            .map(|v| format!("{v} ms"))
            .unwrap_or_else(|| "未知".into()),
        memory = memory_kb
            .map(|v| format!("{v} KB"))
            .unwrap_or_else(|| "未知".into()),
        evidence = evidence.join("\n"),
        statement = truncate_chars(statement, 6_000),
        code = truncate_chars(code, 18_000),
    )
}

async fn collect_evidence(
    db: &Db,
    problem_id: &str,
    verdict: &str,
    time_ms: Option<i64>,
) -> Vec<String> {
    let mut evidence = vec![format!("平台判定：{verdict}")];
    if let Some(t) = time_ms {
        evidence.push(format!("运行时间：{t} ms"));
    }
    if verdict == "CE" {
        evidence.push("编译失败：请优先阅读编译器错误信息".into());
    }
    let row = sqlx::query(
        "SELECT status, stdout, expected, diff, sample_index FROM run_results
         WHERE problem_id = ?1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(problem_id)
    .fetch_optional(&db.pool)
    .await
    .ok()
    .flatten();
    if let Some(row) = row {
        let status: String = row.get("status");
        if status != "OK" {
            let index: i64 = row.get("sample_index");
            let diff: Option<String> = row.get("diff");
            evidence.push(format!("最近一次本地样例 {index} 结果：{status}"));
            if let Some(diff) = diff {
                if !diff.is_empty() {
                    evidence.push(format!("样例差异：{}", truncate_chars(&diff, 600)));
                }
            }
        }
    }
    evidence
}

fn deterministic_analysis(
    title: &str,
    verdict: &str,
    tags: &[String],
    evidence: &[String],
    time_ms: Option<i64>,
) -> Value {
    let category = match verdict {
        "AC" => "none",
        "TLE" | "MLE" => "complexity",
        "CE" => "implementation",
        "RE" => "implementation",
        "WA" | "PE" => "algorithm",
        "OLE" | "ILE" => "io",
        _ => "unknown",
    };
    let summary = if verdict == "AC" {
        format!("{title} 已通过。")
    } else {
        format!("{title} 未通过（{verdict}），需要定位具体失败原因。")
    };
    let root_cause: String = match verdict {
        "AC" => "代码通过全部测试。请检查复杂度是否贴近限制，并考虑用更一般的做法复述本题。".into(),
        "WA" => "输出与期望不一致。优先检查边界条件、题型假设、整数范围与多测清空。".into(),
        "TLE" => {
            "运行时间超过限制。检查时间复杂度、常数因子、是否重复计算或使用了过慢的 I/O。".into()
        }
        "MLE" => "内存超过限制。检查是否开了过大的数组/容器，或递归栈过深。".into(),
        "RE" => "程序运行时崩溃。检查数组越界、空指针、除零、递归深度与未初始化变量。".into(),
        "CE" => "编译失败。先修复语法/类型错误，再检查是否使用了不受支持的扩展。".into(),
        _ => "提交未通过，需要结合平台反馈与本地样例进一步定位。".into(),
    };
    json!({
        "summary": summary,
        "failure_category": category,
        "root_cause": root_cause,
        "bug_patterns": [],
        "evidence": evidence,
        "complexity": {
            "time": "未分析",
            "space": "未分析",
            "expected": "未知",
            "verdict": if verdict == "TLE" { "too_slow" } else { "unknown" }
        },
        "fix_steps": if verdict == "AC" { Vec::<String>::new() } else {
            vec![
                "用最小反例复现问题：先测 n=1、全相同、极值边界。".into(),
                "对照题面检查输入输出格式、下标范围与整数类型。".into(),
                "重算复杂度；若接近时限，替换数据结构或优化常数。".into(),
            ]
        },
        "next_practice_tags": tags,
        "learning_points": if verdict == "AC" {
            vec!["通过题也要复盘复杂度与可迁移技巧。".to_string()]
        } else {
            vec!["先写暴力对拍，再逐步优化。".to_string()]
        },
        "confidence": 0.15,
        "time_ms": time_ms
    })
}

fn normalize_payload(mut payload: Value) -> Value {
    if !payload.is_object() {
        payload = json!({});
    }
    let allowed = [
        "algorithm",
        "implementation",
        "complexity",
        "edge_case",
        "io",
        "overflow",
        "recursion",
        "unknown",
        "none",
    ];
    let category = payload
        .get("failure_category")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    if !allowed.contains(&category) {
        payload["failure_category"] = json!("unknown");
    }
    if payload.get("summary").and_then(|v| v.as_str()).is_none() {
        payload["summary"] = json!("暂无分析");
    }
    for key in [
        "bug_patterns",
        "evidence",
        "fix_steps",
        "next_practice_tags",
        "learning_points",
    ] {
        if !payload.get(key).map(|v| v.is_array()).unwrap_or(false) {
            payload[key] = json!([]);
        }
    }
    if !payload
        .get("confidence")
        .map(|v| v.is_number())
        .unwrap_or(false)
    {
        payload["confidence"] = json!(0.0);
    }
    if !payload
        .get("complexity")
        .map(|v| v.is_object())
        .unwrap_or(false)
    {
        payload["complexity"] =
            json!({"time":"未知","space":"未知","expected":"未知","verdict":"unknown"});
    }
    payload
}

pub async fn update_skill_profile(
    db: &Db,
    tags: &[String],
    problem_rating: i64,
    verdict: &str,
) -> AppResult<()> {
    if tags.is_empty() {
        return Ok(());
    }
    let score = if is_ac(verdict) { 1.0 } else { 0.0 };
    let rating = problem_rating.clamp(800, 3500) as f64;
    for tag in tags {
        let row = sqlx::query("SELECT rating, attempts, solved FROM skill_profiles WHERE tag = ?1")
            .bind(tag)
            .fetch_optional(&db.pool)
            .await?;
        let (current, attempts, solved) = match row {
            Some(row) => (
                row.get::<f64, _>("rating"),
                row.get::<i64, _>("attempts"),
                row.get::<i64, _>("solved"),
            ),
            None => (1200.0, 0, 0),
        };
        let expected = 1.0 / (1.0 + 10f64.powf((rating - current) / 400.0));
        let delta = 24.0 * (score - expected);
        let next = (current + delta).clamp(400.0, 3500.0);
        sqlx::query(
            "INSERT INTO skill_profiles(tag, rating, attempts, solved, last_delta, updated_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(tag) DO UPDATE SET
               rating = excluded.rating,
               attempts = excluded.attempts,
               solved = excluded.solved,
               last_delta = excluded.last_delta,
               updated_at = excluded.updated_at",
        )
        .bind(tag)
        .bind(next)
        .bind(attempts + 1)
        .bind(solved + if is_ac(verdict) { 1 } else { 0 })
        .bind(delta)
        .bind(now_iso())
        .execute(&db.pool)
        .await?;
    }
    Ok(())
}

pub async fn skill_stats(db: &Db) -> AppResult<Vec<TagStatDto>> {
    let rows = sqlx::query(
        "SELECT tag, rating, attempts, solved FROM skill_profiles ORDER BY rating ASC, attempts DESC",
    )
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| TagStatDto {
            tag: row.get("tag"),
            rating: row.get("rating"),
            attempts: row.get("attempts"),
            solved: row.get("solved"),
        })
        .collect())
}

pub async fn weak_tags(db: &Db, limit: i64) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT tag FROM skill_profiles WHERE attempts > 0 ORDER BY rating ASC, attempts DESC LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows.into_iter().map(|row| row.get("tag")).collect())
}

/// Re-analyze submissions that were left pending (app restart, crash, etc.).
/// Only submissions with stored code are eligible; synced historical
/// submissions have no source and are skipped.
pub async fn resume_pending(app: &AppHandle, db: &Db, llm: &LlmClient) {
    let rows = sqlx::query(
        "SELECT s.id FROM submissions s JOIN submission_code c ON c.submission_id = s.id
         WHERE s.analysis_status IN ('pending', 'running') ORDER BY s.submitted_at DESC LIMIT 5",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap_or_default();
    for row in rows {
        let id: String = row.get("id");
        let app = app.clone();
        let db = db.clone();
        let llm = llm.clone();
        tokio::spawn(async move {
            let _ = analyze_submission(&app, &db, &llm, &id, false).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_bad_payloads() {
        let value = normalize_payload(json!({"failure_category": "nonsense"}));
        assert_eq!(value["failure_category"], "unknown");
        assert!(value["bug_patterns"].is_array());
        assert_eq!(value["confidence"], 0.0);
    }

    #[test]
    fn deterministic_categorizes_verdicts() {
        let v = deterministic_analysis("A", "TLE", &[], &[], Some(2000));
        assert_eq!(v["failure_category"], "complexity");
        let v = deterministic_analysis("A", "AC", &[], &[], Some(10));
        assert_eq!(v["failure_category"], "none");
    }
}
