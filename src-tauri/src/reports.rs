use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::llm::LlmClient;
use crate::models::{GenerateReportRequest, ReportDto, ReportSummaryDto};
use crate::provider;
use crate::util::{is_ac, now_iso, truncate_chars};
use chrono::{Duration, NaiveDate, SecondsFormat, TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::Row;
use tauri::{AppHandle, Emitter};

const REPORTER_SYSTEM: &str = r#"你是 ICPC 训练教练，负责写一份给选手本人看的阶段训练报告。
风格：具体、诚实、可执行；引用数据而不是空泛鼓励。
结构（Markdown）：
# 标题
## 一、阶段概览（rating/通过数/提交数/通过率）
## 二、表现亮点
## 三、主要问题（按严重程度排序，引用提交与分析证据）
## 四、标签能力变化
## 五、下一阶段训练建议（3-5 条，具体到标签与题目类型）
## 六、本周行动清单
使用简体中文。不要编造没有的数据。不要输出 JSON。"#;

pub async fn generate_report(
    app: &AppHandle,
    db: &Db,
    llm: &LlmClient,
    req: GenerateReportRequest,
) -> AppResult<ReportDto> {
    let (start, end) = period_bounds(&req)?;
    let data = build_data(db, &start, &end).await?;
    let provider_id;
    let model_id;
    let mut content = None;
    match provider::resolve_role(db, "report", &provider::session_for("report")).await {
        Ok(resolved) => {
            provider_id = Some(resolved.provider_id.clone());
            model_id = Some(resolved.model.clone());
            let prompt = build_prompt(&req.kind, &start, &end, &data);
            match llm
                .complete_text(
                    &resolved.config,
                    &resolved.model,
                    REPORTER_SYSTEM,
                    &prompt,
                    4_000,
                )
                .await
            {
                Ok(text) if !text.trim().is_empty() => content = Some(text),
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!("report model failed: {err}");
                }
            }
        }
        Err(_) => {
            provider_id = None;
            model_id = None;
        }
    }
    let content = content.unwrap_or_else(|| fallback_markdown(&req.kind, &start, &end, &data));
    let title = format!(
        "ICPC 训练报告 · {} · {}",
        kind_label(&req.kind),
        &start[..10]
    );
    let id = uuid::Uuid::now_v7().to_string();
    let created_at = now_iso();
    sqlx::query(
        "INSERT INTO reports(id, kind, title, period_start, period_end, content_md, data_json, provider_id, model_id, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .bind(&id)
    .bind(&req.kind)
    .bind(&title)
    .bind(&start)
    .bind(&end)
    .bind(&content)
    .bind(data.to_string())
    .bind(&provider_id)
    .bind(&model_id)
    .bind(&created_at)
    .execute(&db.pool)
    .await?;
    let _ = app.emit("report://updated", json!({ "id": id }));
    get_report(db, &id).await
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "weekly" => "周报",
        "monthly" => "月报",
        "contest" => "比赛复盘",
        _ => "阶段总结",
    }
}

fn period_bounds(req: &GenerateReportRequest) -> AppResult<(String, String)> {
    let parse = |s: &Option<String>, fallback: NaiveDate| -> NaiveDate {
        s.as_deref()
            .and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok())
            .unwrap_or(fallback)
    };
    let today = Utc::now().date_naive();
    let default_start = today - Duration::days(7);
    let start_date = parse(&req.period_start, default_start);
    let end_date = parse(&req.period_end, today);
    if start_date > end_date {
        return Err(AppError::invalid("报告开始日期必须早于结束日期"));
    }
    let start = Utc
        .from_utc_datetime(&start_date.and_hms_opt(0, 0, 0).unwrap())
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    let end = Utc
        .from_utc_datetime(&end_date.and_hms_opt(23, 59, 59).unwrap())
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    Ok((start, end))
}

async fn build_data(db: &Db, start: &str, end: &str) -> AppResult<Value> {
    let accounts = sqlx::query(
        "SELECT platform, handle, uid, rating, max_rating, solved_count, last_synced_at FROM platform_accounts ORDER BY platform",
    )
    .fetch_all(&db.pool)
    .await?;
    let accounts: Vec<Value> = accounts
        .into_iter()
        .map(|row| {
            json!({
                "platform": row.get::<String, _>("platform"),
                "handle": row.get::<Option<String>, _>("handle"),
                "rating": row.get::<Option<i64>, _>("rating").unwrap_or(0),
                "max_rating": row.get::<Option<i64>, _>("max_rating").unwrap_or(0),
                "solved": row.get::<Option<i64>, _>("solved_count").unwrap_or(0),
                "last_synced_at": row.get::<Option<String>, _>("last_synced_at")
            })
        })
        .collect();
    let submissions = sqlx::query(
        "SELECT s.verdict, s.platform, s.language, s.time_ms, s.submitted_at, p.title, p.normalized_rating, p.skill_tags_json
         FROM submissions s JOIN problems p ON p.id = s.problem_id
         WHERE s.submitted_at BETWEEN ?1 AND ?2 ORDER BY s.submitted_at DESC",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&db.pool)
    .await?;
    let mut total = 0i64;
    let mut accepted = 0i64;
    let mut sub_rows = Vec::new();
    for row in submissions {
        total += 1;
        let verdict: String = row.get("verdict");
        if is_ac(&verdict) {
            accepted += 1;
        }
        sub_rows.push(json!({
            "title": row.get::<String, _>("title"),
            "platform": row.get::<String, _>("platform"),
            "verdict": verdict,
            "rating": row.get::<Option<i64>, _>("normalized_rating"),
            "tags": row.get::<Option<String>, _>("skill_tags_json").and_then(|s| serde_json::from_str::<Value>(&s).ok()).unwrap_or(json!([])),
            "at": row.get::<String, _>("submitted_at"),
            "time_ms": row.get::<Option<i64>, _>("time_ms")
        }));
    }
    let analyses = sqlx::query(
        "SELECT s.submitted_at, s.verdict, p.title, a.payload_json
         FROM analyses a JOIN submissions s ON s.id = a.submission_id
         JOIN problems p ON p.id = s.problem_id
         WHERE s.submitted_at BETWEEN ?1 AND ?2 ORDER BY s.submitted_at DESC LIMIT 20",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&db.pool)
    .await?;
    let analyses: Vec<Value> = analyses
        .into_iter()
        .map(|row| {
            let payload: Value =
                serde_json::from_str(row.get::<&str, _>("payload_json")).unwrap_or(json!({}));
            json!({
                "title": row.get::<String, _>("title"),
                "verdict": row.get::<String, _>("verdict"),
                "at": row.get::<String, _>("submitted_at"),
                "summary": payload.get("summary"),
                "category": payload.get("failure_category"),
                "root_cause": payload.get("root_cause"),
                "next_practice_tags": payload.get("next_practice_tags")
            })
        })
        .collect();
    let tags = crate::analysis::skill_stats(db).await?;
    let plan = crate::plan::active_plan(db).await?;
    let plan_progress = plan.as_ref().map(|p| {
        let done = p.items.iter().filter(|i| i.status == "done").count();
        json!({ "total": p.items.len(), "done": done, "rationale": p.rationale })
    });
    Ok(json!({
        "period": { "start": start, "end": end },
        "accounts": accounts,
        "stats": {
            "submissions": total,
            "accepted": accepted,
            "acceptance_rate": if total > 0 { accepted as f64 / total as f64 } else { 0.0 }
        },
        "submissions": sub_rows.into_iter().take(50).collect::<Vec<_>>(),
        "analyses": analyses,
        "skill_tags": tags,
        "active_plan": plan_progress
    }))
}

fn build_prompt(kind: &str, start: &str, end: &str, data: &Value) -> String {
    format!(
        "报告类型：{}\n统计区间：{} 至 {}\n\n训练数据（JSON）：\n{}\n\n请生成报告。",
        kind_label(kind),
        start,
        end,
        truncate_chars(&data.to_string(), 18_000)
    )
}

fn fallback_markdown(kind: &str, start: &str, end: &str, data: &Value) -> String {
    let stats = data.get("stats").cloned().unwrap_or(json!({}));
    let total = stats
        .get("submissions")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let accepted = stats.get("accepted").and_then(|v| v.as_i64()).unwrap_or(0);
    let rate = stats
        .get("acceptance_rate")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let mut weak = Vec::new();
    if let Some(tags) = data.get("skill_tags").and_then(|v| v.as_array()) {
        for tag in tags.iter().take(5) {
            weak.push(format!(
                "{}（{:.0}，{} 次）",
                tag.get("tag").and_then(|v| v.as_str()).unwrap_or("?"),
                tag.get("rating").and_then(|v| v.as_f64()).unwrap_or(0.0),
                tag.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0)
            ));
        }
    }
    let mut rows = String::new();
    if let Some(subs) = data.get("submissions").and_then(|v| v.as_array()) {
        for sub in subs.iter().take(12) {
            rows.push_str(&format!(
                "| {} | {} | {} |\n",
                sub.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                sub.get("platform").and_then(|v| v.as_str()).unwrap_or(""),
                sub.get("verdict").and_then(|v| v.as_str()).unwrap_or("")
            ));
        }
    }
    format!(
        r#"# ICPC 训练报告 · {}

统计区间：{} 至 {}

## 一、阶段概览

- 提交数：{}
- 通过数：{}
- 通过率：{:.1}%

## 二、能力画像

薄弱标签：{}

## 三、近期提交

| 题目 | 平台 | 结果 |
|---|---|---|
{}

## 四、下一阶段建议

1. 先复盘最近一次未通过的题，写出最小反例。
2. 针对最弱标签连续做 3 道同标签、难度递增的题。
3. 每道题提交前至少跑一遍样例与一组自造边界数据。
4. 通过题也记录复杂度与可迁移技巧。

> 本报告由本地规则生成（未配置模型或模型调用失败）。配置 Agent 后可以获得更具体的代码级分析。
"#,
        kind_label(kind),
        start,
        end,
        total,
        accepted,
        rate * 100.0,
        if weak.is_empty() {
            "暂无（多提交几道带标签的题后会自动统计）".to_string()
        } else {
            weak.join("、")
        },
        rows
    )
}

pub async fn list_reports(db: &Db) -> AppResult<Vec<ReportSummaryDto>> {
    let rows = sqlx::query(
        "SELECT id, kind, title, period_start, period_end, created_at FROM reports ORDER BY created_at DESC LIMIT 100",
    )
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ReportSummaryDto {
            id: row.get("id"),
            kind: row.get("kind"),
            title: row.get("title"),
            period_start: row.get("period_start"),
            period_end: row.get("period_end"),
            created_at: row.get("created_at"),
        })
        .collect())
}

pub async fn get_report(db: &Db, id: &str) -> AppResult<ReportDto> {
    let row = sqlx::query(
        "SELECT id, kind, title, period_start, period_end, content_md, data_json, created_at FROM reports WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(&db.pool)
    .await?
    .ok_or_else(|| AppError::not_found("报告"))?;
    Ok(ReportDto {
        id: row.get("id"),
        kind: row.get("kind"),
        title: row.get("title"),
        period_start: row.get("period_start"),
        period_end: row.get("period_end"),
        content_md: row.get("content_md"),
        data: serde_json::from_str(row.get::<&str, _>("data_json")).unwrap_or(json!({})),
        created_at: row.get("created_at"),
    })
}

pub async fn delete_report(db: &Db, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM reports WHERE id = ?1")
        .bind(id)
        .execute(&db.pool)
        .await?;
    Ok(())
}
