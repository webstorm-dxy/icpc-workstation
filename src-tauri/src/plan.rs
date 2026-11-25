use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::llm::LlmClient;
use crate::models::{GeneratePlanRequest, PlanDto, PlanItemDto, PlatformQuota, SettingsDto};
use crate::provider;
use crate::util::{now_iso, parse_json_array};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::Row;
use tauri::{AppHandle, Emitter};

const PLANNER_SYSTEM: &str = r#"你是 ICPC 训练规划师，也是一名代码审查教练。
你必须先分析给定的最近提交、代码（如果有）和既有分析，再制定训练计划。
要求：
0. 先做诊断：指出具体的错误模式、边界/实现缺陷、薄弱标签和证据，引用题号、判定或代码片段。代码为 null 时必须明确说明只有判定/标签证据，绝不能编造代码内容。
1. 只输出一个 JSON 对象，不要 markdown 代码块。
2. diagnosis 写基于提交与代码的诊断；rationale 写训练方案总体思路；items 是题目列表。
3. problem_id 必须来自候选列表；不能选择已经 AC 的题；attempted_before=true 的题只能作为 review/复盘。
4. 至少覆盖 4 个不同的标签；不要连续 3 题同一标签。
5. 至少 1 道 review（做过但没通过）或 1 道 stretch（难度高于目标 rating 150-300）。
6. reason 要具体说明“为什么是这道题、练什么、对应诊断里的哪个问题”，不要写空话。
7. 全部文本用简体中文。
8. 如果给出了「题目来源配额」，必须严格按配额选题：每个平台的题目数量要精确等于配额，不能多也不能少；在配额内再考虑难度与标签匹配。"#;

fn plan_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["diagnosis", "rationale", "items"],
        "properties": {
            "diagnosis": {"type": "string", "description": "基于最近提交与代码的诊断：错误模式、证据、薄弱点"},
            "rationale": {"type": "string", "description": "本次训练方案的总体思路"},
            "items": {
                "type": "array",
                "minItems": 4,
                "maxItems": 12,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["problem_id", "kind", "reason", "target_tags", "expected_minutes"],
                    "properties": {
                        "problem_id": {"type": "string"},
                        "kind": {"type": "string", "enum": ["practice", "review", "stretch"]},
                        "reason": {"type": "string"},
                        "target_tags": {"type": "array", "items": {"type": "string"}},
                        "expected_minutes": {"type": "integer", "minimum": 15, "maximum": 240}
                    }
                }
            }
        }
    })
}

#[derive(Debug, Clone)]
struct Candidate {
    id: String,
    platform: String,
    title: String,
    rating: Option<i64>,
    normalized: i64,
    skill_tags: Vec<String>,
    attempted: bool,
    score: f64,
}

/// Sanitised `(platform, count)` pairs. Unknown platforms and zero counts are
/// dropped, duplicates merge to the larger value, and the total is capped to
/// the 12 items the plan schema accepts.
fn normalize_quotas(platforms: Option<&Vec<PlatformQuota>>) -> Vec<(String, i64)> {
    let mut out: Vec<(String, i64)> = Vec::new();
    let Some(list) = platforms else {
        return out;
    };
    for item in list {
        let platform = item.platform.trim().to_ascii_lowercase();
        if !crate::platforms::PLATFORMS.contains(&platform.as_str()) {
            continue;
        }
        let count = item.count.clamp(0, 12);
        if count == 0 {
            continue;
        }
        match out.iter_mut().find(|(existing, _)| *existing == platform) {
            Some(existing) => existing.1 = existing.1.max(count),
            None => out.push((platform, count)),
        }
    }
    loop {
        let total: i64 = out.iter().map(|(_, count)| *count).sum();
        if total <= 12 {
            break;
        }
        let index = out
            .iter()
            .enumerate()
            .max_by_key(|(_, (_, count))| *count)
            .map(|(index, _)| index);
        match index {
            Some(index) => out[index].1 -= 1,
            None => break,
        }
        out.retain(|(_, count)| *count > 0);
    }
    out
}

fn quota_total(quotas: &[(String, i64)]) -> i64 {
    quotas.iter().map(|(_, count)| *count).sum()
}

fn quota_summary(quotas: &[(String, i64)], items: &[PickedItem]) -> String {
    quotas
        .iter()
        .map(|(platform, quota)| {
            let have = items
                .iter()
                .filter(|item| &item.platform == platform)
                .count();
            format!(
                "{} {}/{} 道",
                crate::platforms::platform_name(platform),
                have,
                quota
            )
        })
        .collect::<Vec<_>>()
        .join("，")
}

/// Keeps roughly five candidates per requested slot for every platform, then
/// fills the rest of the (bounded) pool so the agent still sees alternatives.
fn build_pool(mut candidates: Vec<Candidate>, quotas: &[(String, i64)]) -> Vec<Candidate> {
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    if quotas.is_empty() {
        candidates.truncate(60);
        return candidates;
    }
    let mut pool: Vec<Candidate> = Vec::new();
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (platform, count) in quotas {
        let mut budget = (*count * 5).max(8) as usize;
        for candidate in candidates.iter().filter(|c| &c.platform == platform) {
            if budget == 0 {
                break;
            }
            if taken.insert(candidate.id.clone()) {
                pool.push(candidate.clone());
                budget -= 1;
            }
        }
    }
    let mut rest: Vec<Candidate> = candidates
        .into_iter()
        .filter(|candidate| !taken.contains(&candidate.id))
        .collect();
    rest.truncate(60usize.saturating_sub(pool.len()));
    pool.extend(rest);
    pool.sort_by(|a, b| b.score.total_cmp(&a.score));
    pool
}

pub async fn active_plan(db: &Db) -> AppResult<Option<PlanDto>> {
    let row = sqlx::query(
        "SELECT id FROM training_plans WHERE status = 'active' ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_optional(&db.pool)
    .await?;
    match row {
        Some(row) => Ok(Some(
            plan_dto(db, row.get::<String, _>("id").as_str()).await?,
        )),
        None => Ok(None),
    }
}

pub async fn plan_dto(db: &Db, plan_id: &str) -> AppResult<PlanDto> {
    let row = sqlx::query(
        "SELECT id, status, rationale_md, created_at FROM training_plans WHERE id = ?1",
    )
    .bind(plan_id)
    .fetch_optional(&db.pool)
    .await?
    .ok_or_else(|| AppError::not_found("训练计划"))?;
    let item_rows = sqlx::query(
        "SELECT i.id, i.problem_id, i.kind, i.reason, i.target_tags_json, i.expected_minutes, i.status,
                p.title, p.platform, p.url, p.rating, p.tags_json
         FROM plan_items i JOIN problems p ON p.id = i.problem_id
         WHERE i.plan_id = ?1 ORDER BY i.rowid ASC",
    )
    .bind(plan_id)
    .fetch_all(&db.pool)
    .await?;
    let items = item_rows
        .into_iter()
        .map(|row| PlanItemDto {
            id: row.get("id"),
            problem_id: row.get("problem_id"),
            title: row.get("title"),
            platform: row.get("platform"),
            url: row.get("url"),
            rating: row.get("rating"),
            tags: parse_json_array(Some(row.get::<&str, _>("tags_json").to_string())),
            kind: row
                .get::<Option<String>, _>("kind")
                .unwrap_or_else(|| "practice".into()),
            reason: row.get::<Option<String>, _>("reason").unwrap_or_default(),
            expected_minutes: row.get("expected_minutes"),
            status: row.get("status"),
        })
        .collect();
    Ok(PlanDto {
        id: row.get("id"),
        status: row.get("status"),
        rationale: row
            .get::<Option<String>, _>("rationale_md")
            .unwrap_or_default(),
        created_at: row.get("created_at"),
        items,
    })
}

pub async fn set_item_status(db: &Db, item_id: &str, status: &str) -> AppResult<()> {
    sqlx::query(
        "UPDATE plan_items SET status = ?1, completed_at = CASE WHEN ?1 = 'done' THEN ?2 ELSE NULL END WHERE id = ?3",
    )
    .bind(status)
    .bind(now_iso())
    .bind(item_id)
    .execute(&db.pool)
    .await?;
    Ok(())
}

fn emit_agent(app: Option<&AppHandle>, value: Value) {
    if let Some(app) = app {
        let _ = app.emit("plan://agent", value);
    }
}

fn emit_agent_error(app: Option<&AppHandle>, trigger: &str, err: &AppError) {
    emit_agent(
        app,
        json!({
            "phase": "error",
            "trigger": trigger,
            "code": err.code,
            "message": err.message,
        }),
    );
}

pub async fn generate_plan(
    app: &AppHandle,
    db: &Db,
    llm: &LlmClient,
    req: GeneratePlanRequest,
) -> AppResult<PlanDto> {
    generate_plan_impl(Some(app), db, llm, req, "manual", false).await
}

pub async fn generate_plan_for_refresh(
    app: &AppHandle,
    db: &Db,
    llm: &LlmClient,
    req: GeneratePlanRequest,
) -> AppResult<PlanDto> {
    generate_plan_impl(Some(app), db, llm, req, "auto", false).await
}

/// Only for the demo seed. Normal generation must go through the agent.
pub async fn generate_demo_plan(
    app: &AppHandle,
    db: &Db,
    llm: &LlmClient,
    req: GeneratePlanRequest,
) -> AppResult<PlanDto> {
    generate_plan_impl(Some(app), db, llm, req, "demo", true).await
}

async fn generate_plan_impl(
    app: Option<&AppHandle>,
    db: &Db,
    llm: &LlmClient,
    req: GeneratePlanRequest,
    trigger: &str,
    allow_fallback: bool,
) -> AppResult<PlanDto> {
    emit_agent(
        app,
        json!({
            "phase": "start",
            "trigger": trigger,
            "message": "读取最近提交、代码与能力画像…"
        }),
    );
    let mut quotas = normalize_quotas(req.platforms.as_ref());
    // A plan below four items is not a plan; grow the largest quota instead of
    // failing the request.
    let requested: i64 = quota_total(&quotas);
    if !quotas.is_empty() && requested < 4 {
        if let Some(widest) = quotas.iter_mut().max_by_key(|(_, count)| *count) {
            widest.1 += 4 - requested;
        }
    }
    let count = if quotas.is_empty() {
        req.count.unwrap_or(8).clamp(4, 12)
    } else {
        quota_total(&quotas).clamp(4, 12)
    };
    let target = target_rating(db).await?;
    let weak = if let Some(tags) = req.focus_tags.clone().filter(|t| !t.is_empty()) {
        tags
    } else {
        crate::analysis::weak_tags(db, 8).await?
    };
    let evidence = load_evidence(db).await?;
    let mut candidates = load_candidates(db, target, &weak).await?;
    if candidates.len() < 10 {
        candidates = load_candidates_wide(db, target, &weak).await?;
    }
    // A requested platform whose whole pool sits outside the rating band would
    // otherwise silently come back as "0/N 道"; widen the band just for it.
    if !quotas.is_empty() {
        let mut seen: std::collections::HashSet<String> =
            candidates.iter().map(|c| c.id.clone()).collect();
        for (platform, _) in &quotas {
            if candidates.iter().any(|c| &c.platform == platform) {
                continue;
            }
            for candidate in load_candidates_for(db, platform, 600, 3800, target, &weak).await? {
                if seen.insert(candidate.id.clone()) {
                    candidates.push(candidate);
                }
            }
        }
    }
    let candidates = build_pool(candidates, &quotas);
    if candidates.is_empty() {
        let err = AppError::new(
            "E_PLAN_NO_CANDIDATES",
            "还没有可推荐的题目，请先同步至少一个 OJ 的题池",
        );
        emit_agent_error(app, trigger, &err);
        return Err(err);
    }

    let reason_text = req.reason.clone().unwrap_or_else(|| "训练计划".into());
    let mut provider_id = None;
    let mut model_id = None;
    let (rationale, items) =
        match provider::resolve_role(db, "recommend", &provider::session_for("plan")).await {
            Ok(resolved) => {
                provider_id = Some(resolved.provider_id.clone());
                model_id = Some(resolved.model.clone());
                emit_agent(
                    app,
                    json!({
                        "phase": "model",
                        "trigger": trigger,
                        "provider": resolved.provider_id,
                        "model": resolved.model,
                        "message": "Agent 正在分析提交与代码，再挑选题目…"
                    }),
                );
                let prompt = build_prompt(
                    target,
                    &weak,
                    &candidates,
                    count,
                    &reason_text,
                    &evidence,
                    &quotas,
                )
                .await?;
                let mut on_delta = |delta: &str| {
                    emit_agent(
                        app,
                        json!({
                            "phase": "output",
                            "trigger": trigger,
                            "delta": delta
                        }),
                    );
                };
                let streamed = llm
                    .complete_json_stream(
                        &resolved.config,
                        &resolved.model,
                        PLANNER_SYSTEM,
                        &prompt,
                        plan_schema(),
                        8_000,
                        &mut on_delta,
                    )
                    .await;
                let value = match streamed {
                    Ok(value) => Some(value),
                    Err(stream_err) => {
                        emit_agent(
                            app,
                            json!({
                                "phase": "model",
                                "trigger": trigger,
                                "provider": provider_id,
                                "model": model_id,
                                "message": "流式输出中断，正在用同一 Agent 重试…"
                            }),
                        );
                        match llm
                            .complete_json(
                                &resolved.config,
                                &resolved.model,
                                PLANNER_SYSTEM,
                                &prompt,
                                plan_schema(),
                                8_000,
                            )
                            .await
                        {
                            Ok(value) => {
                                if let Ok(raw) = serde_json::to_string(&value) {
                                    emit_agent(
                                        app,
                                        json!({
                                            "phase": "output",
                                            "trigger": trigger,
                                            "delta": raw
                                        }),
                                    );
                                }
                                Some(value)
                            }
                            Err(retry_err) => {
                                if allow_fallback {
                                    None
                                } else {
                                    let wrapped = AppError::new(
                                        "E_PLAN_AGENT",
                                        format!(
                                            "Agent 生成训练计划失败：{}；重试也失败：{}",
                                            stream_err.message, retry_err.message
                                        ),
                                    )
                                    .with_detail(format!(
                                        "{}\n--- retry ---\n{}",
                                        stream_err.detail.unwrap_or_default(),
                                        retry_err.detail.unwrap_or_default()
                                    ));
                                    emit_agent_error(app, trigger, &wrapped);
                                    return Err(wrapped);
                                }
                            }
                        }
                    }
                };
                match value {
                    Some(value) => {
                        match normalize_payload(value, &candidates, count, &quotas, &weak) {
                            Ok(pair) => pair,
                            Err(reason) => {
                                if allow_fallback {
                                    deterministic_plan(&candidates, &weak, count, &quotas)
                                } else {
                                    let err = AppError::new(
                                        "E_PLAN_OUTPUT",
                                        format!("Agent 返回的训练计划无效：{reason}"),
                                    );
                                    emit_agent_error(app, trigger, &err);
                                    return Err(err);
                                }
                            }
                        }
                    }
                    None => deterministic_plan(&candidates, &weak, count, &quotas),
                }
            }
            Err(err) => {
                if allow_fallback {
                    deterministic_plan(&candidates, &weak, count, &quotas)
                } else {
                    let wrapped = AppError::new(
                        "E_PLAN_PROVIDER",
                        "生成训练计划需要 Agent 推荐模型，请先在设置中配置提供商和“推荐”角色",
                    )
                    .with_detail(err.message);
                    emit_agent_error(app, trigger, &wrapped);
                    return Err(wrapped);
                }
            }
        };

    let plan_id = uuid::Uuid::now_v7().to_string();
    let created_at = now_iso();
    let mut tx = db.pool.begin().await?;
    sqlx::query("UPDATE training_plans SET status = 'superseded' WHERE status = 'active'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO training_plans(id, status, reason, source_submission_id, rationale_md, provider_id, model_id, created_at)
         VALUES(?1, 'active', ?2, NULL, ?3, ?4, ?5, ?6)",
    )
    .bind(&plan_id)
    .bind(&reason_text)
    .bind(&rationale)
    .bind(&provider_id)
    .bind(&model_id)
    .bind(&created_at)
    .execute(&mut *tx)
    .await?;
    for item in &items {
        sqlx::query(
            "INSERT INTO plan_items(id, plan_id, problem_id, kind, reason, target_tags_json, expected_minutes, status)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending')",
        )
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(&plan_id)
        .bind(&item.problem_id)
        .bind(&item.kind)
        .bind(&item.reason)
        .bind(serde_json::to_string(&item.target_tags).unwrap_or_else(|_| "[]".into()))
        .bind(item.expected_minutes)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    // Remember the requested mix so the post-submission refresh keeps it.
    if req.platforms.is_some() {
        let stored: Vec<PlatformQuota> = quotas
            .iter()
            .map(|(platform, count)| PlatformQuota {
                platform: platform.clone(),
                count: *count,
            })
            .collect();
        let _ = db.set_json("plan.quotas", &stored).await;
    }
    emit_agent(
        app,
        json!({
            "phase": "done",
            "trigger": trigger,
            "planId": plan_id,
            "message": "训练计划已生成"
        }),
    );
    plan_dto(db, &plan_id).await
}

async fn target_rating(db: &Db) -> AppResult<i64> {
    let row = sqlx::query(
        "SELECT MAX(COALESCE(rating, 0)) AS rating FROM platform_accounts WHERE platform IN ('codeforces','atcoder')",
    )
    .fetch_one(&db.pool)
    .await?;
    let rating = row.get::<Option<i64>, _>("rating").unwrap_or(0);
    Ok(if rating <= 0 { 1200 } else { rating })
}

async fn load_candidates(db: &Db, target: i64, weak: &[String]) -> AppResult<Vec<Candidate>> {
    load_candidates_range(db, target - 300, target + 500, target, weak).await
}

async fn load_candidates_wide(db: &Db, target: i64, weak: &[String]) -> AppResult<Vec<Candidate>> {
    load_candidates_range(db, 600, 3800, target, weak).await
}

async fn load_candidates_range(
    db: &Db,
    min: i64,
    max: i64,
    target: i64,
    weak: &[String],
) -> AppResult<Vec<Candidate>> {
    let mut out = Vec::new();
    for platform in crate::platforms::PLATFORMS {
        out.extend(load_candidates_for(db, platform, min, max, target, weak).await?);
    }
    Ok(out)
}

/// One platform at a time on purpose: a large pool (Codeforces routinely has
/// more than ten thousand problems) would otherwise fill the single capped
/// query and AtCoder or Luogu would never reach the quota logic.
async fn load_candidates_for(
    db: &Db,
    platform: &str,
    min: i64,
    max: i64,
    target: i64,
    weak: &[String],
) -> AppResult<Vec<Candidate>> {
    let active_ids: Vec<String> = sqlx::query(
        "SELECT i.problem_id FROM plan_items i JOIN training_plans t ON t.id = i.plan_id
         WHERE t.status = 'active'",
    )
    .fetch_all(&db.pool)
    .await?
    .into_iter()
    .map(|row| row.get::<String, _>("problem_id"))
    .collect();
    let rows = sqlx::query(
        "SELECT p.id, p.platform, p.title, p.rating, p.normalized_rating, p.tags_json, p.skill_tags_json,
            EXISTS(SELECT 1 FROM submissions s WHERE s.problem_id = p.id AND s.verdict = 'AC') AS solved,
            EXISTS(SELECT 1 FROM submissions s WHERE s.problem_id = p.id) AS attempted
         FROM problems p
         WHERE p.platform = ?4 AND p.normalized_rating BETWEEN ?1 AND ?2
         ORDER BY ABS(p.normalized_rating - ?3) ASC
         LIMIT 250",
    )
    .bind(min)
    .bind(max)
    .bind(target)
    .bind(platform)
    .fetch_all(&db.pool)
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let solved = row.get::<i64, _>("solved") != 0;
        if solved {
            continue;
        }
        let id: String = row.get("id");
        if active_ids.contains(&id) {
            continue;
        }
        let skill_tags: Vec<String> =
            parse_json_array(Some(row.get::<&str, _>("skill_tags_json").to_string()));
        let normalized = row.get::<i64, _>("normalized_rating");
        let attempted = row.get::<i64, _>("attempted") != 0;
        let overlap = skill_tags.iter().filter(|tag| weak.contains(tag)).count() as f64;
        let proximity = 100.0 - ((normalized - target).abs().min(600) as f64 / 6.0);
        let mut score = overlap * 38.0 + proximity;
        if attempted {
            score += 12.0;
        }
        // Stable pseudo-jitter so repeated generations do not return the exact same order.
        let hash = crate::util::sha256_hex(&id);
        let jitter = u8::from_str_radix(&hash[..2], 16).unwrap_or(0) as f64 / 255.0 * 4.0;
        score += jitter;
        out.push(Candidate {
            id,
            platform: row.get("platform"),
            title: row.get("title"),
            rating: row.get("rating"),
            normalized,
            skill_tags,
            attempted,
            score,
        });
    }
    Ok(out)
}

async fn load_evidence(db: &Db) -> AppResult<Value> {
    let account_rows = sqlx::query(
        "SELECT platform, handle, rating, max_rating, solved_count FROM platform_accounts ORDER BY platform",
    )
    .fetch_all(&db.pool)
    .await?;
    let accounts: Vec<Value> = account_rows
        .into_iter()
        .map(|row| {
            json!({
                "platform": row.get::<String, _>("platform"),
                "handle": row.get::<Option<String>, _>("handle"),
                "rating": row.get::<Option<i64>, _>("rating").unwrap_or(0),
                "max_rating": row.get::<Option<i64>, _>("max_rating").unwrap_or(0),
                "solved_count": row.get::<Option<i64>, _>("solved_count").unwrap_or(0),
            })
        })
        .collect();
    let skill_rows = sqlx::query(
        "SELECT tag, rating, attempts, solved FROM skill_profiles ORDER BY rating ASC LIMIT 12",
    )
    .fetch_all(&db.pool)
    .await?;
    let skills: Vec<Value> = skill_rows
        .into_iter()
        .map(|row| {
            json!({
                "tag": row.get::<String, _>("tag"),
                "elo": row.get::<f64, _>("rating"),
                "attempts": row.get::<i64, _>("attempts"),
                "solved": row.get::<i64, _>("solved"),
            })
        })
        .collect();

    let rows = sqlx::query(
        "SELECT s.id, s.platform, s.problem_id, s.verdict, s.time_ms, s.submitted_at, s.language,
                p.title, p.rating, p.normalized_rating, p.tags_json,
                sc.code_text, a.payload_json
         FROM submissions s
         JOIN problems p ON p.id = s.problem_id
         LEFT JOIN submission_code sc ON sc.submission_id = s.id
         LEFT JOIN analyses a ON a.submission_id = s.id
         ORDER BY s.submitted_at DESC LIMIT 30",
    )
    .fetch_all(&db.pool)
    .await?;

    struct RowData {
        id: String,
        platform: String,
        problem_id: String,
        verdict: String,
        time_ms: Option<i64>,
        submitted_at: String,
        language: Option<String>,
        title: String,
        rating: Option<i64>,
        normalized_rating: Option<i64>,
        tags: Vec<String>,
        code: Option<String>,
        analysis: Option<Value>,
    }

    let mut items = Vec::new();
    for row in rows {
        let code: Option<String> = row.get("code_text");
        let analysis = row
            .get::<Option<String>, _>("payload_json")
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
        items.push(RowData {
            id: row.get("id"),
            platform: row.get("platform"),
            problem_id: row.get("problem_id"),
            verdict: row.get("verdict"),
            time_ms: row.get("time_ms"),
            submitted_at: row.get("submitted_at"),
            language: row.get("language"),
            title: row.get("title"),
            rating: row.get("rating"),
            normalized_rating: row.get("normalized_rating"),
            tags: parse_json_array(Some(row.get::<&str, _>("tags_json").to_string())),
            code,
            analysis,
        });
    }

    // Show source for at most 8 submissions, prioritising failures over AC.
    let mut code_slots = std::collections::HashSet::new();
    for failed_pass in [true, false] {
        for item in &items {
            if code_slots.len() >= 8 {
                break;
            }
            let has_code = item
                .code
                .as_deref()
                .map(|code| !code.trim().is_empty())
                .unwrap_or(false);
            if has_code && ((item.verdict != "AC") == failed_pass) {
                code_slots.insert(item.id.clone());
            }
        }
    }
    let submissions_with_code = items
        .iter()
        .filter(|item| {
            item.code
                .as_deref()
                .map(|code| !code.trim().is_empty())
                .unwrap_or(false)
        })
        .count();
    let mut recent = Vec::with_capacity(items.len());
    for item in items {
        let include_code = code_slots.contains(&item.id);
        let code = item
            .code
            .as_deref()
            .filter(|_| include_code)
            .map(|code| crate::util::truncate_chars(code, 3_000));
        recent.push(json!({
            "submission_id": item.id,
            "platform": item.platform,
            "problem_id": item.problem_id,
            "title": item.title,
            "rating": item.rating,
            "normalized_rating": item.normalized_rating,
            "tags": item.tags,
            "verdict": item.verdict,
            "time_ms": item.time_ms,
            "submitted_at": item.submitted_at,
            "language": item.language,
            "code": code,
            "analysis": item.analysis,
        }));
    }

    Ok(json!({
        "accounts": accounts,
        "skill_tags_weakest_first": skills,
        "recent_submissions": recent,
        "submissions_with_code": submissions_with_code,
        "code_included_in_context": code_slots.len(),
        "note": if submissions_with_code == 0 {
            "没有可用的提交源码；只能依据判定、题目标签和既有分析诊断。不要编造代码。"
        } else {
            "code 为 null 表示该提交没有源码或未纳入本轮上下文；有代码的提交应优先分析实现细节。"
        },
    }))
}

async fn build_prompt(
    target: i64,
    weak: &[String],
    candidates: &[Candidate],
    count: i64,
    reason: &str,
    evidence: &Value,
    quotas: &[(String, i64)],
) -> AppResult<String> {
    let candidate_json = candidates
        .iter()
        .map(|c| {
            json!({
                "problem_id": c.id,
                "platform": c.platform,
                "title": c.title,
                "rating": c.rating,
                "normalized_rating": c.normalized,
                "tags": c.skill_tags,
                "attempted_before": c.attempted
            })
        })
        .collect::<Vec<_>>();
    let quota_instruction = if quotas.is_empty() {
        String::new()
    } else {
        format!(
            "\n题目来源配额（必须严格遵守，items 里每个平台的数量要精确等于该数字，不能多也不能少）：{}。共 {} 道。\n",
            quotas
                .iter()
                .map(|(platform, count)| format!(
                    "{} {} 道",
                    crate::platforms::platform_name(platform),
                    count
                ))
                .collect::<Vec<_>>()
                .join("，"),
            count
        )
    };
    Ok(format!(
        r#"训练目标 rating：{target}
本次触发原因：{reason}
薄弱标签（从弱到强）：{weak}
最近提交、代码与既有分析（JSON）：
{evidence}
候选题目（problem_id 只能从这里选）：
{candidates}
{quota}
请先分析上面的提交与代码，在 diagnosis 中指出具体的错误模式、边界/实现缺陷、薄弱标签和证据；如果 code 为 null，明确说明只有判定/标签证据。不要粘贴大段代码，只引用关键片段。diagnosis 控制在 600 字内，rationale 控制在 300 字内，每个 reason 控制在 120 字内。然后给出 rationale 与 items，选出 {count} 道题（至少 4 道），输出符合要求的 JSON。items 的 kind 只能用 practice/review/stretch。如果给了来源配额，挑选时先在每个平台的配额内比较难度与标签匹配，再给出跨平台的整体训练顺序。"#,
        weak = weak.join(", "),
        evidence = serde_json::to_string(evidence).unwrap_or_else(|_| "{}".into()),
        candidates = serde_json::to_string(&candidate_json).unwrap_or_else(|_| "[]".into()),
        quota = quota_instruction,
    ))
}

struct PickedItem {
    problem_id: String,
    platform: String,
    kind: String,
    reason: String,
    target_tags: Vec<String>,
    expected_minutes: i64,
}

fn default_picked(candidate: &Candidate, kind: &str, weak: &[String]) -> PickedItem {
    let chinese_kind = match kind {
        "review" => "复盘",
        "stretch" => "拉伸",
        _ => "练习",
    };
    let tags = if candidate.skill_tags.is_empty() {
        weak.iter().take(2).cloned().collect::<Vec<_>>()
    } else {
        candidate.skill_tags.clone()
    };
    PickedItem {
        problem_id: candidate.id.clone(),
        platform: candidate.platform.clone(),
        kind: kind.to_string(),
        reason: format!(
            "{chinese_kind}：{}（难度 {}，平台 {}），覆盖 {}，匹配当前薄弱点。",
            candidate.title,
            candidate.normalized,
            crate::platforms::platform_name(&candidate.platform),
            if tags.is_empty() {
                "综合".to_string()
            } else {
                tags.join("/")
            }
        ),
        target_tags: tags,
        expected_minutes: if candidate.normalized > 1800 { 70 } else { 45 },
    }
}

fn normalize_payload(
    value: Value,
    candidates: &[Candidate],
    count: i64,
    quotas: &[(String, i64)],
    weak: &[String],
) -> Result<(String, Vec<PickedItem>), String> {
    let map: std::collections::HashMap<&str, &Candidate> =
        candidates.iter().map(|c| (c.id.as_str(), c)).collect();
    let diagnosis = value
        .get("diagnosis")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if diagnosis.is_empty() {
        return Err("缺少基于提交与代码的 diagnosis".into());
    }
    let rationale = value
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let mut remaining: std::collections::HashMap<&str, i64> = quotas
        .iter()
        .map(|(platform, quota)| (platform.as_str(), *quota))
        .collect();
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(arr) = value.get("items").and_then(|v| v.as_array()) {
        for item in arr {
            let problem_id = item
                .get("problem_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let Some(candidate) = map.get(problem_id) else {
                continue;
            };
            if !seen.insert(problem_id.to_string()) {
                continue;
            }
            if !quotas.is_empty() {
                // Off-mix platforms and over-filled quotas are dropped; the
                // shortfall is topped up below.
                match remaining.get_mut(candidate.platform.as_str()) {
                    Some(left) if *left > 0 => *left -= 1,
                    _ => continue,
                }
            }
            let kind = match item.get("kind").and_then(|v| v.as_str()) {
                Some("review") => "review",
                Some("stretch") => "stretch",
                _ => "practice",
            };
            let target_tags = item
                .get("target_tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                })
                .filter(|v: &Vec<String>| !v.is_empty())
                .unwrap_or_else(|| candidate.skill_tags.clone());
            let expected_minutes = item
                .get("expected_minutes")
                .and_then(|v| v.as_i64())
                .unwrap_or(45)
                .clamp(15, 240);
            let chinese_kind = match kind {
                "review" => "复盘",
                "stretch" => "拉伸",
                _ => "练习",
            };
            items.push(PickedItem {
                problem_id: candidate.id.clone(),
                platform: candidate.platform.clone(),
                kind: kind.to_string(),
                reason: item
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        format!(
                            "{chinese_kind}：{}（{}），覆盖 {}",
                            candidate.title,
                            candidate.normalized,
                            target_tags.join("/")
                        )
                    }),
                target_tags,
                expected_minutes,
            });
            if items.len() >= count as usize {
                break;
            }
        }
    }

    // Top up any platform the agent under-filled, so the mix is honoured even
    // when the model ignores the quota.
    let mut shortfalls: Vec<String> = Vec::new();
    if !quotas.is_empty() {
        for (platform, quota) in quotas {
            let mut have = items
                .iter()
                .filter(|item| &item.platform == platform)
                .count() as i64;
            if have >= *quota {
                continue;
            }
            for candidate in candidates.iter().filter(|c| &c.platform == platform) {
                if have >= *quota || items.len() >= count as usize {
                    break;
                }
                if !seen.insert(candidate.id.clone()) {
                    continue;
                }
                items.push(default_picked(candidate, "practice", weak));
                have += 1;
            }
            if have < *quota {
                shortfalls.push(format!(
                    "{} 的候选题目不足，实际安排 {have}/{} 道，差额由其他平台补齐",
                    crate::platforms::platform_name(platform),
                    quota
                ));
            }
        }
        // Never hand back a plan shorter than requested just because one
        // platform ran dry.
        if items.len() < count as usize {
            for candidate in candidates {
                if items.len() >= count as usize {
                    break;
                }
                if !seen.insert(candidate.id.clone()) {
                    continue;
                }
                items.push(default_picked(candidate, "practice", weak));
            }
        }
    }

    if items.len() < 4 {
        let detail = if shortfalls.is_empty() {
            String::new()
        } else {
            format!("（{}）", shortfalls.join("；"))
        };
        return Err(format!(
            "只解析出 {} 道有效题目，至少需要 4 道{detail}",
            items.len()
        ));
    }

    let quota_block = if quotas.is_empty() {
        String::new()
    } else {
        let mut text = format!("\n\n> 题目来源：{}。", quota_summary(quotas, &items));
        for note in &shortfalls {
            text.push_str(&format!("\n> {note}。"));
        }
        text
    };
    let rationale = format!(
        "## Agent 诊断\n\n{diagnosis}\n\n## 训练方案\n\n{}{}",
        if rationale.is_empty() {
            "根据诊断选择上述题目。"
        } else {
            &rationale
        },
        quota_block
    );
    Ok((rationale, items))
}

fn pick_into(
    pool: &mut Vec<Candidate>,
    take: usize,
    items: &mut Vec<PickedItem>,
    used_tags: &mut std::collections::HashSet<String>,
    used_ids: &mut std::collections::HashSet<String>,
    weak: &[String],
) {
    let mut taken = 0;
    while taken < take && !pool.is_empty() {
        // Prefer a candidate that adds a tag not covered yet.
        let index = pool
            .iter()
            .position(|candidate| {
                candidate
                    .skill_tags
                    .iter()
                    .any(|tag| !used_tags.contains(tag))
            })
            .unwrap_or(0);
        let candidate = pool.remove(index);
        if !used_ids.insert(candidate.id.clone()) {
            continue;
        }
        for tag in &candidate.skill_tags {
            used_tags.insert(tag.clone());
        }
        let kind = if candidate.attempted {
            "review"
        } else if candidate.normalized > 1700 && items.iter().all(|item| item.kind != "stretch") {
            "stretch"
        } else {
            "practice"
        };
        items.push(default_picked(&candidate, kind, weak));
        taken += 1;
    }
}

fn deterministic_plan(
    candidates: &[Candidate],
    weak: &[String],
    count: i64,
    quotas: &[(String, i64)],
) -> (String, Vec<PickedItem>) {
    let mut sorted = candidates.to_vec();
    sorted.sort_by(|a, b| b.score.total_cmp(&a.score));
    let target = count as usize;
    let mut items: Vec<PickedItem> = Vec::new();
    let mut used_tags = std::collections::HashSet::new();
    let mut used_ids = std::collections::HashSet::new();

    if quotas.is_empty() {
        let mut pool = sorted;
        pick_into(
            &mut pool,
            target,
            &mut items,
            &mut used_tags,
            &mut used_ids,
            weak,
        );
    } else {
        for (platform, quota) in quotas {
            let mut pool: Vec<Candidate> = sorted
                .iter()
                .filter(|candidate| &candidate.platform == platform)
                .cloned()
                .collect();
            pick_into(
                &mut pool,
                *quota as usize,
                &mut items,
                &mut used_tags,
                &mut used_ids,
                weak,
            );
        }
        if items.len() < target {
            let mut pool: Vec<Candidate> = sorted
                .iter()
                .filter(|candidate| !used_ids.contains(&candidate.id))
                .cloned()
                .collect();
            let missing = target - items.len();
            pick_into(
                &mut pool,
                missing,
                &mut items,
                &mut used_tags,
                &mut used_ids,
                weak,
            );
        }
    }

    let mut rationale = format!(
        "根据当前 rating 与薄弱标签（{}）生成的确定性训练方案。优先选择未做过的题，并保留一道复盘或拉伸题。",
        weak.join("、")
    );
    if !quotas.is_empty() {
        rationale.push_str(&format!(
            "\n\n> 题目来源：{}。",
            quota_summary(quotas, &items)
        ));
    }
    (rationale, items)
}

pub fn spawn_refresh(
    app: AppHandle,
    db: Db,
    llm: LlmClient,
    source_submission_id: Option<String>,
    _problem_id: Option<String>,
) {
    tokio::spawn(async move {
        let settings = db
            .get_json::<SettingsDto>("settings")
            .await
            .unwrap_or_default();
        if !settings.auto_refresh_plan {
            return;
        }
        // Debounce: do not regenerate more than once every 20 seconds.
        let last = sqlx::query("SELECT MAX(created_at) AS at FROM training_plans")
            .fetch_one(&db.pool)
            .await
            .ok()
            .and_then(|row| row.get::<Option<String>, _>("at"));
        if let Some(at) = last {
            if let Ok(dt) = DateTime::parse_from_rfc3339(&at) {
                let age = Utc::now().signed_duration_since(dt.with_timezone(&Utc));
                if age.num_seconds() < 20 {
                    return;
                }
            }
        }
        let quotas: Vec<PlatformQuota> = db.get_json("plan.quotas").await.unwrap_or_default();
        let req = GeneratePlanRequest {
            reason: Some("提交后自动刷新训练方案".into()),
            focus_tags: None,
            count: Some(8),
            platforms: (!quotas.is_empty()).then_some(quotas),
        };
        match generate_plan_for_refresh(&app, &db, &llm, req).await {
            Ok(plan) => {
                let _ = app.emit(
                    "plan://updated",
                    json!({ "plan": plan, "sourceSubmissionId": source_submission_id }),
                );
            }
            Err(err) => {
                let _ = app.emit(
                    "plan://error",
                    json!({ "message": err.message, "code": err.code, "sourceSubmissionId": source_submission_id }),
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str) -> Candidate {
        Candidate {
            id: id.to_string(),
            platform: "codeforces".into(),
            title: id.to_string(),
            rating: Some(1100),
            normalized: 1100,
            skill_tags: vec!["dp".into()],
            attempted: false,
            score: 0.0,
        }
    }

    fn platform_candidate(id: &str, platform: &str, score: f64) -> Candidate {
        Candidate {
            id: id.to_string(),
            platform: platform.into(),
            title: id.to_string(),
            rating: Some(1200),
            normalized: 1200,
            skill_tags: vec![format!("tag-{platform}")],
            attempted: false,
            score,
        }
    }

    /// Regression: a 11k-problem Codeforces pool used to fill the single
    /// distance-ordered query, so Luogu contributed 0 candidates and every
    /// "洛谷 N 道" quota came back as "0/N 道".
    #[tokio::test]
    async fn small_platform_survives_a_huge_codeforces_pool() {
        let dir = std::env::temp_dir().join(format!("icpc-plan-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let db = Db::open(&dir.join("test.db")).await.expect("db");
        for index in 0..700 {
            sqlx::query(
                "INSERT INTO problems(id, platform, native_id, title, url, normalized_rating, tags_json, skill_tags_json, samples_json)
                 VALUES(?1, 'codeforces', ?2, ?2, 'https://example.com', 1050, '[]', '[]', '[]')",
            )
            .bind(format!("cf:{index}"))
            .bind(format!("C{index}"))
            .execute(&db.pool)
            .await
            .expect("insert");
        }
        for index in 0..5 {
            sqlx::query(
                "INSERT INTO problems(id, platform, native_id, title, url, normalized_rating, tags_json, skill_tags_json, samples_json)
                 VALUES(?1, 'luogu', ?2, ?2, 'https://example.com', 1500, '[]', '[]', '[]')",
            )
            .bind(format!("luogu:P{index}"))
            .bind(format!("P{index}"))
            .execute(&db.pool)
            .await
            .expect("insert");
        }

        let candidates = load_candidates(&db, 1053, &[]).await.expect("candidates");
        let luogu = candidates.iter().filter(|c| c.platform == "luogu").count();
        assert_eq!(luogu, 5, "Luogu must not be crowded out by Codeforces");
        let pool = build_pool(candidates, &[("luogu".to_string(), 4)]);
        assert!(pool.iter().filter(|c| c.platform == "luogu").count() >= 4);

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_platform_quotas_from_the_ui_payload() {
        let req: GeneratePlanRequest = serde_json::from_value(json!({
            "reason": "训练页手动生成",
            "platforms": [
                {"platform": "codeforces", "count": 4},
                {"platform": "luogu", "count": 2}
            ]
        }))
        .expect("camelCase payload");
        let quotas = normalize_quotas(req.platforms.as_ref());
        assert_eq!(quota_total(&quotas), 6);
        assert_eq!(quotas[1], ("luogu".to_string(), 2));

        // An explicit empty list means "no mix", not "missing".
        let req: GeneratePlanRequest =
            serde_json::from_value(json!({"platforms": []})).expect("empty list");
        assert!(req.platforms.is_some());
        assert!(normalize_quotas(req.platforms.as_ref()).is_empty());
    }

    #[test]
    fn normalizes_platform_quotas() {
        let quotas = normalize_quotas(Some(&vec![
            PlatformQuota {
                platform: " Codeforces ".into(),
                count: 3,
            },
            PlatformQuota {
                platform: "luogu".into(),
                count: 2,
            },
            PlatformQuota {
                platform: "codeforces".into(),
                count: 1,
            },
            PlatformQuota {
                platform: "nowhere".into(),
                count: 5,
            },
            PlatformQuota {
                platform: "atcoder".into(),
                count: 0,
            },
        ]));
        assert_eq!(
            quotas,
            vec![("codeforces".to_string(), 3), ("luogu".to_string(), 2)]
        );

        // Totals above the schema limit are trimmed largest-first.
        let capped = normalize_quotas(Some(&vec![
            PlatformQuota {
                platform: "codeforces".into(),
                count: 12,
            },
            PlatformQuota {
                platform: "luogu".into(),
                count: 12,
            },
        ]));
        assert_eq!(quota_total(&capped), 12);
        assert!(capped.iter().all(|(_, count)| *count >= 6));
    }

    #[test]
    fn build_pool_keeps_requested_platforms() {
        let mut candidates = Vec::new();
        for index in 0..20 {
            candidates.push(platform_candidate(
                &format!("cf:{index}"),
                "codeforces",
                100.0,
            ));
        }
        for index in 0..3 {
            candidates.push(platform_candidate(&format!("luogu:P{index}"), "luogu", 1.0));
        }
        let pool = build_pool(
            candidates,
            &[("codeforces".to_string(), 2), ("luogu".to_string(), 2)],
        );
        let luogu = pool.iter().filter(|c| c.platform == "luogu").count();
        assert_eq!(luogu, 3, "low-scoring platform must still reach the pool");
        let codeforces = pool.iter().filter(|c| c.platform == "codeforces").count();
        assert!(codeforces >= 10, "quota platforms get a 5x budget");
        assert!(pool.len() <= 60);
    }

    #[test]
    fn enforces_quotas_on_agent_output() {
        let mut candidates = Vec::new();
        for index in 0..4 {
            candidates.push(platform_candidate(
                &format!("cf:{index}"),
                "codeforces",
                10.0,
            ));
            candidates.push(platform_candidate(&format!("luogu:P{index}"), "luogu", 1.0));
        }
        let quotas = vec![("codeforces".to_string(), 2), ("luogu".to_string(), 2)];
        let item = |id: &str| {
            json!({
                "problem_id": id,
                "kind": "practice",
                "reason": "覆盖边界情况",
                "target_tags": ["dp"],
                "expected_minutes": 45
            })
        };
        // The agent ignores the quota and only returns Codeforces problems.
        let value = json!({
            "diagnosis": "边界处理不稳。",
            "rationale": "先巩固边界。",
            "items": [item("cf:0"), item("cf:1"), item("cf:2"), item("cf:3")]
        });
        let (rationale, items) =
            normalize_payload(value, &candidates, 4, &quotas, &[]).expect("valid plan");
        assert_eq!(items.len(), 4);
        let codeforces = items.iter().filter(|i| i.platform == "codeforces").count();
        let luogu = items.iter().filter(|i| i.platform == "luogu").count();
        assert_eq!((codeforces, luogu), (2, 2));
        assert!(rationale.contains("Codeforces 2/2 道"));
        assert!(rationale.contains("洛谷 2/2 道"));
    }

    #[test]
    fn reports_platform_shortfall() {
        let candidates = vec![
            platform_candidate("cf:0", "codeforces", 10.0),
            platform_candidate("cf:1", "codeforces", 9.0),
            platform_candidate("cf:2", "codeforces", 8.0),
            platform_candidate("cf:3", "codeforces", 7.0),
        ];
        let quotas = vec![("codeforces".to_string(), 3), ("luogu".to_string(), 2)];
        let value = json!({
            "diagnosis": "诊断",
            "rationale": "方案",
            "items": []
        });
        let (rationale, items) =
            normalize_payload(value, &candidates, 5, &quotas, &[]).expect("still valid");
        assert_eq!(
            items.len(),
            4,
            "shortfall is topped up from other platforms"
        );
        assert!(rationale.contains("洛谷 的候选题目不足"));
        assert!(rationale.contains("实际安排 0/2 道"));
    }

    #[test]
    fn deterministic_plan_honours_quotas() {
        let candidates = vec![
            platform_candidate("cf:0", "codeforces", 10.0),
            platform_candidate("cf:1", "codeforces", 9.0),
            platform_candidate("luogu:P0", "luogu", 8.0),
            platform_candidate("luogu:P1", "luogu", 7.0),
            platform_candidate("cf:2", "codeforces", 6.0),
        ];
        let quotas = vec![("codeforces".to_string(), 2), ("luogu".to_string(), 1)];
        let (rationale, items) = deterministic_plan(&candidates, &["dp".to_string()], 3, &quotas);
        assert_eq!(items.len(), 3);
        assert_eq!(
            items.iter().filter(|i| i.platform == "codeforces").count(),
            2
        );
        assert_eq!(items.iter().filter(|i| i.platform == "luogu").count(), 1);
        assert!(rationale.contains("题目来源"));
    }

    #[test]
    fn requires_agent_diagnosis_and_at_least_four_items() {
        let candidates = vec![
            candidate("cf:1"),
            candidate("cf:2"),
            candidate("cf:3"),
            candidate("cf:4"),
        ];
        let item = |id: &str| {
            json!({
                "problem_id": id,
                "kind": "practice",
                "reason": "针对代码中的边界错误",
                "target_tags": ["dp"],
                "expected_minutes": 45
            })
        };
        let valid = json!({
            "diagnosis": "最近提交显示边界处理不稳。",
            "rationale": "先巩固边界。",
            "items": [item("cf:1"), item("cf:2"), item("cf:3"), item("cf:4")]
        });
        let (rationale, items) =
            normalize_payload(valid, &candidates, 4, &[], &[]).expect("valid plan");
        assert!(rationale.contains("Agent 诊断"));
        assert_eq!(items.len(), 4);

        let missing_diagnosis = json!({
            "rationale": "没有诊断",
            "items": [item("cf:1"), item("cf:2"), item("cf:3"), item("cf:4")]
        });
        assert!(normalize_payload(missing_diagnosis, &candidates, 4, &[], &[]).is_err());

        let too_short = json!({
            "diagnosis": "诊断",
            "rationale": "方案",
            "items": [item("cf:1")]
        });
        assert!(normalize_payload(too_short, &candidates, 4, &[], &[]).is_err());
    }
}
