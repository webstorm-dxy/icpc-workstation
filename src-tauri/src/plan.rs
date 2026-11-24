use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::llm::LlmClient;
use crate::models::{GeneratePlanRequest, PlanDto, PlanItemDto, SettingsDto};
use crate::provider;
use crate::util::{now_iso, parse_json_array};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::Row;
use tauri::{AppHandle, Emitter};

const PLANNER_SYSTEM: &str = r#"你是 ICPC 训练规划师。你只从给定的候选题目里选择，绝不编造题号。
目标：让选手在接下来 1-2 周内，以 60% 巩固薄弱点、40% 适度拉伸的配比训练。
要求：
1. 只输出一个 JSON 对象，不要 markdown 代码块。
2. problem_id 必须来自候选列表，且不能是已经做过的题。
3. 至少覆盖 4 个不同的薄弱标签；不要连续 3 题同一标签。
4. 至少 1 道 review（做过但没通过的题）或 1 道 stretch（难度高于目标 rating 150-300）。
5. reason 要具体说明“为什么是这道题、练什么”，不要写空话。
6. 全部文本用简体中文。"#;

fn plan_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["rationale", "items"],
        "properties": {
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

pub async fn generate_plan(
    db: &Db,
    llm: &LlmClient,
    req: GeneratePlanRequest,
) -> AppResult<PlanDto> {
    let settings = db
        .get_json::<SettingsDto>("settings")
        .await
        .unwrap_or_default();
    let count = req.count.unwrap_or(8).clamp(4, 12);
    let target = target_rating(db).await?;
    let weak = if let Some(tags) = req.focus_tags.clone().filter(|t| !t.is_empty()) {
        tags
    } else {
        crate::analysis::weak_tags(db, 8).await?
    };
    let mut candidates = load_candidates(db, target, &weak).await?;
    if candidates.len() < 10 {
        candidates = load_candidates_wide(db, target, &weak).await?;
    }
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    candidates.truncate(60);
    if candidates.is_empty() {
        return Err(AppError::new(
            "E_PLAN_NO_CANDIDATES",
            "还没有可推荐的题目，请先同步至少一个 OJ 的题池",
        ));
    }

    let mut provider_id = None;
    let mut model_id = None;
    let mut payload: Option<Value> = None;
    match provider::resolve_role(db, "recommend", &provider::session_for("plan")).await {
        Ok(resolved) => {
            provider_id = Some(resolved.provider_id.clone());
            model_id = Some(resolved.model.clone());
            let prompt = build_prompt(
                db,
                target,
                &weak,
                &candidates,
                count,
                req.reason.as_deref().unwrap_or("手动生成"),
            )
            .await?;
            match llm
                .complete_json(
                    &resolved.config,
                    &resolved.model,
                    PLANNER_SYSTEM,
                    &prompt,
                    plan_schema(),
                    3_000,
                )
                .await
            {
                Ok(value) => payload = Some(value),
                Err(err) => {
                    tracing::warn!("recommend model failed: {err}");
                }
            }
        }
        Err(err) => {
            tracing::info!("no recommend provider, using deterministic plan: {err}");
        }
    }

    let (rationale, items) = match payload {
        Some(value) => normalize_payload(value, &candidates, count),
        None => deterministic_plan(&candidates, &weak, count),
    };
    let items = if items.len() < 4 {
        deterministic_plan(&candidates, &weak, count).1
    } else {
        items
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
    .bind(req.reason.clone().unwrap_or_else(|| "训练计划".into()))
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
    let _ = settings;
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
         WHERE p.normalized_rating BETWEEN ?1 AND ?2
         ORDER BY ABS(p.normalized_rating - ?3) ASC
         LIMIT 700",
    )
    .bind(min)
    .bind(max)
    .bind(target)
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

async fn build_prompt(
    db: &Db,
    target: i64,
    weak: &[String],
    candidates: &[Candidate],
    count: i64,
    reason: &str,
) -> AppResult<String> {
    let recent = sqlx::query(
        "SELECT s.verdict, s.time_ms, s.submitted_at, p.title, p.normalized_rating
         FROM submissions s JOIN problems p ON p.id = s.problem_id
         ORDER BY s.submitted_at DESC LIMIT 12",
    )
    .fetch_all(&db.pool)
    .await?;
    let recent: Vec<Value> = recent
        .into_iter()
        .map(|row| {
            json!({
                "title": row.get::<String, _>("title"),
                "verdict": row.get::<String, _>("verdict"),
                "rating": row.get::<Option<i64>, _>("normalized_rating"),
                "at": row.get::<String, _>("submitted_at")
            })
        })
        .collect();
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
    Ok(format!(
        r#"训练目标 rating：{target}
本次触发原因：{reason}
薄弱标签（从弱到强）：{weak}
最近 12 次提交：{recent}
候选题目（只能从这里选）：
{candidates}

请选出 {count} 道题，输出 rationale 与 items。items 的 kind 只能用 practice/review/stretch。"#,
        weak = weak.join(", "),
        recent = serde_json::to_string(&recent).unwrap_or_else(|_| "[]".into()),
        candidates = serde_json::to_string(&candidate_json).unwrap_or_else(|_| "[]".into()),
    ))
}

struct PickedItem {
    problem_id: String,
    kind: String,
    reason: String,
    target_tags: Vec<String>,
    expected_minutes: i64,
}

fn normalize_payload(
    value: Value,
    candidates: &[Candidate],
    count: i64,
) -> (String, Vec<PickedItem>) {
    let map: std::collections::HashMap<&str, &Candidate> =
        candidates.iter().map(|c| (c.id.as_str(), c)).collect();
    let rationale = value
        .get("rationale")
        .and_then(|v| v.as_str())
        .unwrap_or("根据近期表现与薄弱标签生成的训练方案。")
        .to_string();
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
    (rationale, items)
}

fn deterministic_plan(
    candidates: &[Candidate],
    weak: &[String],
    count: i64,
) -> (String, Vec<PickedItem>) {
    let mut sorted = candidates.to_vec();
    sorted.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut items: Vec<PickedItem> = Vec::new();
    let mut used_tags: std::collections::HashSet<String> = std::collections::HashSet::new();
    let target = count as usize;
    let mut pool = sorted.clone();
    // Prefer tag diversity first.
    while items.len() < target && !pool.is_empty() {
        let mut picked = None;
        for (idx, candidate) in pool.iter().enumerate() {
            let fresh_tags = candidate
                .skill_tags
                .iter()
                .filter(|tag| !used_tags.contains(*tag))
                .count();
            if fresh_tags > 0 || items.len() + pool.len() <= target {
                picked = Some(idx);
                break;
            }
        }
        let idx = picked.unwrap_or(0);
        let candidate = pool.remove(idx);
        if items.len() < target {
            for tag in &candidate.skill_tags {
                used_tags.insert(tag.clone());
            }
            let kind = if candidate.attempted {
                "review"
            } else if candidate.normalized > 1700 && items.iter().all(|i| i.kind != "stretch") {
                "stretch"
            } else {
                "practice"
            };
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
            items.push(PickedItem {
                problem_id: candidate.id.clone(),
                kind: kind.to_string(),
                reason: format!(
                    "{chinese_kind}：{}（难度 {}，平台 {}），覆盖 {}，匹配当前薄弱点。",
                    candidate.title,
                    candidate.normalized,
                    candidate.platform,
                    tags.join("/")
                ),
                target_tags: tags,
                expected_minutes: if candidate.normalized > 1800 { 70 } else { 45 },
            });
        }
    }
    let rationale = format!(
        "根据当前 rating 与薄弱标签（{}）生成的确定性训练方案。优先选择未做过的题，并保留一道复盘或拉伸题。",
        weak.join("、")
    );
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
        let req = GeneratePlanRequest {
            reason: Some("提交后自动刷新训练方案".into()),
            focus_tags: None,
            count: Some(8),
            platforms: None,
        };
        match generate_plan(&db, &llm, req).await {
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
