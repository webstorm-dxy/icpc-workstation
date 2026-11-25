use crate::analysis;
use crate::error::{AppError, AppResult};
use crate::judge;
use crate::llm::{ChatRequest, LlmEvent, LlmMessage, ToolCall, ToolSpec};
use crate::models::{
    AgentDeltaEvent, AgentDoneEvent, AgentToolEvent, ChatMessageDto, ConversationDto,
    GeneratePlanRequest, GenerateReportRequest, PlatformQuota, RunSamplesRequest,
    SendMessageResult, SubmitRequest, ToolCallDto, ToolConfirmationDto,
};
use crate::plan;
use crate::provider;
use crate::reports;
use crate::state::SharedState;
use crate::submission;
use crate::util::{now_iso, truncate_chars};
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::Row;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

const CHAT_PROMPT: &str = include_str!("../prompts/chat.md");
const MAX_STEPS: usize = 6;

#[derive(Default)]
pub struct AgentService {
    confirmations: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl AgentService {
    pub fn resolve_confirmation(&self, request_id: &str, approved: bool) -> bool {
        let sender = self
            .confirmations
            .lock()
            .expect("confirmation lock")
            .remove(request_id);
        sender.map(|tx| tx.send(approved).is_ok()).unwrap_or(false)
    }

    pub fn cancel(&self, message_id: &str) -> bool {
        let flag = self
            .cancels
            .lock()
            .expect("cancel lock")
            .get(message_id)
            .cloned();
        if let Some(flag) = flag {
            flag.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    fn register_cancel(&self, message_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancels
            .lock()
            .expect("cancel lock")
            .insert(message_id.to_string(), flag.clone());
        flag
    }

    fn clear_cancel(&self, message_id: &str) {
        self.cancels.lock().expect("cancel lock").remove(message_id);
    }
}

pub async fn list_conversations(db: &crate::db::Db) -> AppResult<Vec<ConversationDto>> {
    let rows = sqlx::query(
        "SELECT id, title, created_at, updated_at FROM conversations ORDER BY updated_at DESC LIMIT 100",
    )
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ConversationDto {
            id: row.get("id"),
            title: row.get("title"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

pub async fn create_conversation(db: &crate::db::Db) -> AppResult<ConversationDto> {
    let now = now_iso();
    let id = uuid::Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO conversations(id, title, created_at, updated_at) VALUES(?1, '新对话', ?2, ?2)",
    )
    .bind(&id)
    .bind(&now)
    .execute(&db.pool)
    .await?;
    Ok(ConversationDto {
        id,
        title: "新对话".into(),
        created_at: now.clone(),
        updated_at: now,
    })
}

pub async fn delete_conversation(db: &crate::db::Db, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM messages WHERE conversation_id = ?1")
        .bind(id)
        .execute(&db.pool)
        .await?;
    sqlx::query("DELETE FROM conversations WHERE id = ?1")
        .bind(id)
        .execute(&db.pool)
        .await?;
    Ok(())
}

pub async fn get_messages(
    db: &crate::db::Db,
    conversation_id: &str,
) -> AppResult<Vec<ChatMessageDto>> {
    let rows = sqlx::query(
        "SELECT id, role, content_md, tool_calls_json, usage_json, created_at
         FROM messages WHERE conversation_id = ?1 ORDER BY created_at ASC, rowid ASC",
    )
    .bind(conversation_id)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ChatMessageDto {
            id: row.get("id"),
            role: row.get("role"),
            content: row.get("content_md"),
            created_at: row.get("created_at"),
            tool_calls: serde_json::from_str(row.get::<&str, _>("tool_calls_json"))
                .unwrap_or_default(),
            usage: row
                .get::<Option<String>, _>("usage_json")
                .and_then(|s| serde_json::from_str(&s).ok()),
        })
        .collect())
}

pub async fn send_message(
    state: SharedState,
    app: AppHandle,
    conversation_id: String,
    text: String,
) -> AppResult<SendMessageResult> {
    if text.trim().is_empty() {
        return Err(AppError::invalid("消息不能为空"));
    }
    let conversation_id = if conversation_id.trim().is_empty() {
        create_conversation(&state.db).await?.id
    } else {
        conversation_id
    };
    let user_id = uuid::Uuid::now_v7().to_string();
    let assistant_id = uuid::Uuid::now_v7().to_string();
    let now = now_iso();
    sqlx::query(
        "INSERT INTO messages(id, conversation_id, role, content_md, tool_calls_json, created_at)
         VALUES(?1, ?2, 'user', ?3, '[]', ?4)",
    )
    .bind(&user_id)
    .bind(&conversation_id)
    .bind(&text)
    .bind(&now)
    .execute(&state.db.pool)
    .await?;
    let title: Option<String> = sqlx::query("SELECT title FROM conversations WHERE id = ?1")
        .bind(&conversation_id)
        .fetch_optional(&state.db.pool)
        .await?
        .map(|row| row.get("title"));
    if title.as_deref().unwrap_or("") == "新对话" {
        let new_title = truncate_chars(text.trim(), 36);
        sqlx::query("UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3")
            .bind(new_title)
            .bind(&now)
            .bind(&conversation_id)
            .execute(&state.db.pool)
            .await?;
    } else {
        sqlx::query("UPDATE conversations SET updated_at = ?1 WHERE id = ?2")
            .bind(&now)
            .bind(&conversation_id)
            .execute(&state.db.pool)
            .await?;
    }
    let result = SendMessageResult {
        conversation_id: conversation_id.clone(),
        user_message_id: user_id,
        assistant_message_id: assistant_id.clone(),
    };
    let app_for_task = app.clone();
    let state_for_task = state.clone();
    tokio::spawn(async move {
        run_turn(state_for_task, app_for_task, conversation_id, assistant_id).await;
    });
    Ok(result)
}

async fn run_turn(
    state: SharedState,
    app: AppHandle,
    conversation_id: String,
    assistant_id: String,
) {
    let cancel = state.agent.register_cancel(&assistant_id);
    let outcome = run_inner(&state, &app, &conversation_id, &assistant_id, cancel).await;
    state.agent.clear_cancel(&assistant_id);
    let (content, usage, error, tool_calls) = match outcome {
        Ok(v) => v,
        Err(err) => (
            format!(
                "出了点问题：{}\n\n> {}",
                err.message,
                err.detail.unwrap_or_default()
            ),
            None,
            Some(err.message),
            vec![],
        ),
    };
    let content = if content.trim().is_empty() {
        if error.is_some() {
            content
        } else {
            "（没有生成文本；如果刚执行了工具，可以在工具卡片中查看结果。）".into()
        }
    } else {
        content
    };
    let now = now_iso();
    let _ = sqlx::query(
        "INSERT INTO messages(id, conversation_id, role, content_md, tool_calls_json, usage_json, created_at)
         VALUES(?1, ?2, 'assistant', ?3, ?4, ?5, ?6)",
    )
    .bind(&assistant_id)
    .bind(&conversation_id)
    .bind(&content)
    .bind(serde_json::to_string(&tool_calls).unwrap_or_else(|_| "[]".into()))
    .bind(usage.as_ref().map(|v| v.to_string()))
    .bind(&now)
    .execute(&state.db.pool)
    .await;
    let _ = sqlx::query("UPDATE conversations SET updated_at = ?1 WHERE id = ?2")
        .bind(&now)
        .bind(&conversation_id)
        .execute(&state.db.pool)
        .await;
    let _ = app.emit(
        "agent://done",
        AgentDoneEvent {
            conversation_id,
            message_id: assistant_id,
            content,
            usage,
            error,
        },
    );
}

type TurnResult = (String, Option<Value>, Option<String>, Vec<ToolCallDto>);

async fn run_inner(
    state: &SharedState,
    app: &AppHandle,
    conversation_id: &str,
    assistant_id: &str,
    cancel: Arc<AtomicBool>,
) -> AppResult<TurnResult> {
    let system = build_system_prompt(state).await?;
    let rows = sqlx::query(
        "SELECT role, content_md FROM messages WHERE conversation_id = ?1
         ORDER BY rowid DESC LIMIT 24",
    )
    .bind(conversation_id)
    .fetch_all(&state.db.pool)
    .await?;
    let mut history: Vec<(String, String)> = rows
        .into_iter()
        .map(|row| (row.get("role"), row.get("content_md")))
        .collect();
    history.reverse();
    let mut messages = vec![LlmMessage::system(system)];
    for (role, content) in history {
        if role == "user" {
            messages.push(LlmMessage::user(content));
        } else if role == "assistant" && !content.trim().is_empty() {
            messages.push(LlmMessage::assistant(content, vec![]));
        }
    }
    let session_id = format!("ses_{}", conversation_id.replace('-', ""));
    let resolved = provider::resolve_role(&state.db, "chat", &session_id).await?;
    let tools = tool_specs();
    let mut all_text = String::new();
    let mut tool_summaries: Vec<ToolCallDto> = Vec::new();
    let mut usage: Option<Value> = None;

    for step in 0..MAX_STEPS {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let request = ChatRequest {
            model: resolved.model.clone(),
            messages: messages.clone(),
            tools: tools.clone(),
            temperature: Some(0.4),
            max_tokens: Some(4_096),
            json_mode: false,
            session_id: session_id.clone(),
            cancel: Some(cancel.clone()),
        };
        let app_cb = app.clone();
        let conversation_cb = conversation_id.to_string();
        let message_cb = assistant_id.to_string();
        let mut step_text = String::new();
        let mut callback = |event: LlmEvent| match event {
            LlmEvent::Text(delta) => {
                step_text.push_str(&delta);
                let _ = app_cb.emit(
                    "agent://delta",
                    AgentDeltaEvent {
                        conversation_id: conversation_cb.clone(),
                        message_id: message_cb.clone(),
                        delta,
                    },
                );
            }
            LlmEvent::Reasoning(delta) => {
                let _ = app_cb.emit(
                    "agent://reasoning",
                    AgentDeltaEvent {
                        conversation_id: conversation_cb.clone(),
                        message_id: message_cb.clone(),
                        delta,
                    },
                );
            }
            _ => {}
        };
        let outcome = match state
            .llm
            .chat_stream(&resolved.config, &request, &mut callback)
            .await
        {
            Ok(outcome) => outcome,
            Err(err) => {
                all_text.push_str(&step_text);
                if all_text.trim().is_empty() {
                    return Err(err);
                }
                all_text.push_str(&format!("\n\n> 生成中断：{}", err.message));
                break;
            }
        };
        if step_text.is_empty() {
            all_text.push_str(&outcome.content);
        } else {
            all_text.push_str(&step_text);
        }
        if outcome.usage.is_some() {
            usage = outcome.usage.clone();
        }
        if outcome.finish_reason.as_deref() == Some("cancelled") || cancel.load(Ordering::Relaxed) {
            break;
        }
        if outcome.tool_calls.is_empty() {
            break;
        }
        messages.push(LlmMessage::assistant(
            outcome.content.clone(),
            outcome.tool_calls.clone(),
        ));
        for call in outcome.tool_calls {
            let running = ToolCallDto {
                id: call.id.clone(),
                name: call.name.clone(),
                status: "running".into(),
                summary: tool_running_summary(&call),
            };
            let _ = app.emit(
                "agent://tool",
                AgentToolEvent {
                    conversation_id: conversation_id.to_string(),
                    message_id: assistant_id.to_string(),
                    tool: running,
                },
            );
            let (status, result, summary) = execute_tool(state, app, assistant_id, &call).await;
            let done = ToolCallDto {
                id: call.id.clone(),
                name: call.name.clone(),
                status,
                summary,
            };
            let _ = app.emit(
                "agent://tool",
                AgentToolEvent {
                    conversation_id: conversation_id.to_string(),
                    message_id: assistant_id.to_string(),
                    tool: done.clone(),
                },
            );
            tool_summaries.push(done);
            messages.push(LlmMessage::tool(
                call.id,
                call.name,
                truncate_chars(&result, 12_000),
            ));
        }
        if step + 1 == MAX_STEPS {
            all_text.push_str("\n\n> 已达到本轮工具调用上限。请继续提问以完成后续步骤。");
        }
    }
    Ok((all_text, usage, None, tool_summaries))
}

async fn build_system_prompt(state: &SharedState) -> AppResult<String> {
    let accounts = sqlx::query(
        "SELECT platform, handle, rating, max_rating, solved_count, last_synced_at FROM platform_accounts ORDER BY platform",
    )
    .fetch_all(&state.db.pool)
    .await
    .unwrap_or_default();
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
    let weak = analysis::weak_tags(&state.db, 6).await.unwrap_or_default();
    let plan = plan::active_plan(&state.db).await.ok().flatten();
    let plan_summary = plan.map(|p| {
        json!({
            "rationale": p.rationale,
            "total": p.items.len(),
            "done": p.items.iter().filter(|i| i.status == "done").count()
        })
    });
    let recent = sqlx::query(
        "SELECT s.verdict, p.title, s.submitted_at FROM submissions s JOIN problems p ON p.id = s.problem_id
         ORDER BY s.submitted_at DESC LIMIT 6",
    )
    .fetch_all(&state.db.pool)
    .await
    .unwrap_or_default();
    let recent: Vec<Value> = recent
        .into_iter()
        .map(|row| {
            json!({
                "verdict": row.get::<String, _>("verdict"),
                "title": row.get::<String, _>("title"),
                "at": row.get::<String, _>("submitted_at")
            })
        })
        .collect();
    let snapshot = json!({
        "date": Utc::now().format("%Y-%m-%d").to_string(),
        "accounts": accounts,
        "weak_tags": weak,
        "active_plan": plan_summary,
        "recent_submissions": recent
    });
    Ok(format!(
        "{CHAT_PROMPT}\n\n## 当前训练快照（由应用提供，回答时可直接引用）\n```json\n{}\n```",
        serde_json::to_string_pretty(&snapshot).unwrap_or_else(|_| "{}".into())
    ))
}

fn tool_running_summary(call: &ToolCall) -> String {
    match call.name.as_str() {
        "sync_platform" => "正在同步平台数据…".into(),
        "generate_training_plan" | "refresh_training_plan" => "正在生成训练计划…".into(),
        "run_samples" => "正在本地运行样例…".into(),
        "submit_draft" => "正在准备 OJ 提交页面…".into(),
        "generate_report" => "正在生成报告…".into(),
        "get_training_overview" => "正在读取训练概况…".into(),
        "get_skill_profile" => "正在读取标签能力…".into(),
        "list_candidates" => "正在筛选候选题…".into(),
        "get_problem" => "正在读取题目…".into(),
        "get_recent_submissions" => "正在读取近期提交…".into(),
        _ => "正在执行…".into(),
    }
}

fn tool_specs() -> Vec<ToolSpec> {
    let spec = |name: &str, description: &str, parameters: Value| ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    };
    vec![
        spec(
            "get_training_overview",
            "读取当前账号 rating、通过数、薄弱标签、当前训练计划与最近提交。回答训练状态问题时优先调用。",
            json!({"type":"object","properties":{},"additionalProperties":false}),
        ),
        spec(
            "get_skill_profile",
            "读取各算法标签的 Elo 估计、尝试次数与通过数。",
            json!({"type":"object","properties":{},"additionalProperties":false}),
        ),
        spec(
            "list_candidates",
            "从本地已同步题池中筛选未做过的候选题目，返回题目 ID、标题、难度、标签。生成计划前建议先调用。",
            json!({
                "type":"object",
                "properties":{
                    "platform":{"type":"string","enum":["codeforces","atcoder","luogu"]},
                    "min_rating":{"type":"integer"},
                    "max_rating":{"type":"integer"},
                    "tags":{"type":"array","items":{"type":"string"}},
                    "limit":{"type":"integer","minimum":1,"maximum":40}
                },
                "additionalProperties":false
            }),
        ),
        spec(
            "get_problem",
            "读取一道题的题面摘要、标签、难度与样例数量。",
            json!({"type":"object","properties":{"problem_id":{"type":"string"}},"required":["problem_id"],"additionalProperties":false}),
        ),
        spec(
            "get_recent_submissions",
            "读取最近提交与已有分析摘要，用于复盘。",
            json!({"type":"object","properties":{"limit":{"type":"integer","minimum":1,"maximum":30}},"additionalProperties":false}),
        ),
        spec(
            "generate_training_plan",
            "根据当前画像生成一份新的训练计划（会替换当前 active 计划）。需要用户确认。",
            json!({
                "type":"object",
                "properties":{
                    "reason":{"type":"string"},
                    "focus_tags":{"type":"array","items":{"type":"string"}},
                    "count":{"type":"integer","minimum":4,"maximum":12},
                    "platforms":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["platform","count"],"properties":{"platform":{"type":"string","enum":["codeforces","atcoder","luogu"]},"count":{"type":"integer","minimum":0,"maximum":12}}},"description":"按 OJ 分配题数，例如 [{platform:codeforces,count:4},{platform:luogu,count:2}]；给出后总题数由这些配额之和决定"}
                },
                "additionalProperties":false
            }),
        ),
        spec(
            "refresh_training_plan",
            "按最新提交与能力数据刷新当前训练计划。需要用户确认。",
            json!({"type":"object","properties":{"reason":{"type":"string"}},"additionalProperties":false}),
        ),
        spec(
            "sync_platform",
            "重新执行某个 OJ 的登录校验与数据同步。需要用户确认，耗时较长。",
            json!({"type":"object","properties":{"platform":{"type":"string","enum":["codeforces","atcoder","luogu"]}},"required":["platform"],"additionalProperties":false}),
        ),
        spec(
            "run_samples",
            "在本地编译并运行题目样例。code 省略时使用当前草稿。需要用户确认。",
            json!({"type":"object","properties":{"problem_id":{"type":"string"},"code":{"type":"string"}},"required":["problem_id"],"additionalProperties":false}),
        ),
        spec(
            "submit_draft",
            "把当前草稿填入对应 OJ 的提交页面（不会自动点击提交）。需要用户强确认。",
            json!({"type":"object","properties":{"problem_id":{"type":"string"}},"required":["problem_id"],"additionalProperties":false}),
        ),
        spec(
            "generate_report",
            "生成一份个人训练报告。需要用户确认。",
            json!({"type":"object","properties":{"kind":{"type":"string","enum":["weekly","monthly","stage","contest"]}},"additionalProperties":false}),
        ),
    ]
}

async fn execute_tool(
    state: &SharedState,
    app: &AppHandle,
    assistant_id: &str,
    call: &ToolCall,
) -> (String, String, String) {
    let args = match &call.arguments {
        Value::Object(map) => Value::Object(map.clone()),
        _ => {
            return (
                "error".into(),
                json!({"error":"工具参数不是合法 JSON"}).to_string(),
                "参数格式错误".into(),
            );
        }
    };
    let needs_confirm = matches!(
        call.name.as_str(),
        "generate_training_plan"
            | "refresh_training_plan"
            | "sync_platform"
            | "run_samples"
            | "submit_draft"
            | "generate_report"
    );
    if needs_confirm {
        let summary = confirmation_summary(&call.name, &args);
        let approved =
            request_confirmation(state, app, assistant_id, &call.name, &args, &summary).await;
        if !approved {
            return (
                "denied".into(),
                json!({"error":"用户拒绝了该操作","hint":"如果仍需要，请向用户解释原因并等待再次确认"}).to_string(),
                "用户已拒绝".into(),
            );
        }
    }
    let result = match call.name.as_str() {
        "get_training_overview" => training_overview(state).await,
        "get_skill_profile" => skill_profile(state).await,
        "list_candidates" => list_candidates(state, &args).await,
        "get_problem" => get_problem(state, &args).await,
        "get_recent_submissions" => recent_submissions(state, &args).await,
        "generate_training_plan" | "refresh_training_plan" => {
            generate_plan_tool(state, app, &args, &call.name).await
        }
        "sync_platform" => sync_platform_tool(state, app, &args).await,
        "run_samples" => run_samples_tool(state, &args).await,
        "submit_draft" => submit_draft_tool(state, app, &args).await,
        "generate_report" => generate_report_tool(state, app, &args).await,
        other => Err(AppError::invalid(format!("未知工具: {other}"))),
    };
    match result {
        Ok((summary, payload)) => ("done".into(), payload.to_string(), summary),
        Err(err) => (
            "error".into(),
            json!({"error": err.message, "code": err.code, "detail": err.detail}).to_string(),
            err.message,
        ),
    }
}

fn confirmation_summary(tool: &str, args: &Value) -> String {
    match tool {
        "sync_platform" => format!(
            "同步 {} 的训练数据（会打开内置浏览器，可能需要登录或人机验证）",
            args.get("platform")
                .and_then(|v| v.as_str())
                .unwrap_or("平台")
        ),
        "generate_training_plan" | "refresh_training_plan" => {
            "用当前训练画像生成新的训练计划，并替换当前计划".into()
        }
        "run_samples" => format!(
            "在本地编译并运行题目 {} 的样例",
            args.get("problem_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        ),
        "submit_draft" => format!(
            "打开 OJ 并填好题目 {} 的提交表单；不会自动点击提交",
            args.get("problem_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        ),
        "generate_report" => "生成一份个人训练报告".into(),
        _ => "执行该操作".into(),
    }
}

async fn request_confirmation(
    state: &SharedState,
    app: &AppHandle,
    message_id: &str,
    tool: &str,
    args: &Value,
    summary: &str,
) -> bool {
    let request_id = uuid::Uuid::now_v7().to_string();
    let (tx, rx) = oneshot::channel();
    state
        .agent
        .confirmations
        .lock()
        .expect("confirmation lock")
        .insert(request_id.clone(), tx);
    let _ = app.emit(
        "agent://confirm",
        ToolConfirmationDto {
            request_id: request_id.clone(),
            tool: tool.to_string(),
            summary: summary.to_string(),
            args: args.clone(),
        },
    );
    let _ = message_id;
    let approved = tokio::time::timeout(Duration::from_secs(120), rx)
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or(false);
    state
        .agent
        .confirmations
        .lock()
        .expect("confirmation lock")
        .remove(&request_id);
    approved
}

async fn training_overview(state: &SharedState) -> AppResult<(String, Value)> {
    let accounts = sqlx::query(
        "SELECT platform, handle, rating, max_rating, solved_count, last_synced_at FROM platform_accounts ORDER BY platform",
    )
    .fetch_all(&state.db.pool)
    .await?;
    let accounts: Vec<Value> = accounts
        .into_iter()
        .map(|row| {
            json!({
                "platform": row.get::<String,_>("platform"),
                "handle": row.get::<Option<String>,_>("handle"),
                "rating": row.get::<Option<i64>,_>("rating").unwrap_or(0),
                "max_rating": row.get::<Option<i64>,_>("max_rating").unwrap_or(0),
                "solved": row.get::<Option<i64>,_>("solved_count").unwrap_or(0),
                "last_synced_at": row.get::<Option<String>,_>("last_synced_at")
            })
        })
        .collect();
    let weak = analysis::weak_tags(&state.db, 8).await?;
    let plan = plan::active_plan(&state.db).await?;
    let attempts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM submissions")
        .fetch_one(&state.db.pool)
        .await
        .unwrap_or(0);
    let value = json!({
        "accounts": accounts,
        "weak_tags": weak,
        "submissions": attempts,
        "active_plan": plan.map(|p| json!({"id": p.id, "total": p.items.len(), "done": p.items.iter().filter(|i| i.status=="done").count()})),
    });
    Ok(("已读取训练概况".into(), value))
}

async fn skill_profile(state: &SharedState) -> AppResult<(String, Value)> {
    let stats = analysis::skill_stats(&state.db).await?;
    let weak: Vec<String> = stats.iter().take(5).map(|s| s.tag.clone()).collect();
    Ok((
        format!("共 {} 个标签，最薄弱：{}", stats.len(), weak.join("、")),
        json!({ "tags": stats, "weakest": weak }),
    ))
}

async fn list_candidates(state: &SharedState, args: &Value) -> AppResult<(String, Value)> {
    let platform = args.get("platform").and_then(|v| v.as_str());
    let min = args
        .get("min_rating")
        .and_then(|v| v.as_i64())
        .unwrap_or(600);
    let max = args
        .get("max_rating")
        .and_then(|v| v.as_i64())
        .unwrap_or(3500);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(20)
        .clamp(1, 40);
    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    // The platform filter must run in SQL: with >10k Codeforces rows a global
    // distance-ordered query never reaches AtCoder/Luogu, so filtering
    // afterwards would always answer "0 道".
    let rows = sqlx::query(
        "SELECT p.id, p.title, p.platform, p.rating, p.normalized_rating, p.skill_tags_json,
            EXISTS(SELECT 1 FROM submissions s WHERE s.problem_id = p.id AND s.verdict='AC') AS solved
         FROM problems p
         WHERE (?4 = '' OR p.platform = ?4) AND p.normalized_rating BETWEEN ?1 AND ?2
         ORDER BY ABS(p.normalized_rating - ?3) ASC LIMIT 300",
    )
    .bind(min)
    .bind(max)
    .bind((min + max) / 2)
    .bind(platform.unwrap_or_default())
    .fetch_all(&state.db.pool)
    .await?;
    let mut items = Vec::new();
    for row in rows {
        if row.get::<i64, _>("solved") != 0 {
            continue;
        }
        let row_platform: String = row.get("platform");
        let skill_tags: Vec<String> =
            serde_json::from_str(row.get::<&str, _>("skill_tags_json")).unwrap_or_default();
        if !tags.is_empty() && !tags.iter().any(|t| skill_tags.contains(t)) {
            continue;
        }
        items.push(json!({
            "problem_id": row.get::<String,_>("id"),
            "title": row.get::<String,_>("title"),
            "platform": row_platform,
            "rating": row.get::<Option<i64>,_>("rating"),
            "normalized_rating": row.get::<i64,_>("normalized_rating"),
            "tags": skill_tags
        }));
        if items.len() >= limit as usize {
            break;
        }
    }
    Ok((
        format!("筛选到 {} 道候选题", items.len()),
        json!({ "candidates": items }),
    ))
}

async fn get_problem(state: &SharedState, args: &Value) -> AppResult<(String, Value)> {
    let id = args
        .get("problem_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::invalid("缺少 problem_id"))?;
    let problem = crate::platforms::problem_dto(&state.db, id).await?;
    Ok((
        format!("{}（{}）", problem.title, problem.platform),
        json!({
            "problem_id": problem.id,
            "title": problem.title,
            "platform": problem.platform,
            "rating": problem.rating,
            "normalized_rating": problem.normalized_rating,
            "tags": problem.tags,
            "url": problem.url,
            "samples": problem.samples.len(),
            "solved": problem.solved,
            "attempted": problem.attempted,
            "statement": truncate_chars(&problem.statement_md, 4_000)
        }),
    ))
}

async fn recent_submissions(state: &SharedState, args: &Value) -> AppResult<(String, Value)> {
    let limit = args
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(10)
        .clamp(1, 30);
    let rows = sqlx::query(
        "SELECT s.id, s.verdict, s.submitted_at, p.title, p.platform, a.payload_json
         FROM submissions s JOIN problems p ON p.id = s.problem_id
         LEFT JOIN analyses a ON a.submission_id = s.id
         ORDER BY s.submitted_at DESC LIMIT ?1",
    )
    .bind(limit)
    .fetch_all(&state.db.pool)
    .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let analysis: Option<Value> = row
                .get::<Option<String>, _>("payload_json")
                .and_then(|s| serde_json::from_str(&s).ok());
            json!({
                "submission_id": row.get::<String,_>("id"),
                "title": row.get::<String,_>("title"),
                "platform": row.get::<String,_>("platform"),
                "verdict": row.get::<String,_>("verdict"),
                "at": row.get::<String,_>("submitted_at"),
                "analysis": analysis.map(|a| json!({
                    "summary": a.get("summary"),
                    "category": a.get("failure_category"),
                    "root_cause": a.get("root_cause")
                }))
            })
        })
        .collect();
    Ok((
        format!("读取了 {} 条提交", items.len()),
        json!({ "submissions": items }),
    ))
}

async fn generate_plan_tool(
    state: &SharedState,
    app: &AppHandle,
    args: &Value,
    _tool: &str,
) -> AppResult<(String, Value)> {
    let focus_tags = args
        .get("focus_tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        });
    let count = args.get("count").and_then(|v| v.as_i64());
    let platforms = args
        .get("platforms")
        .and_then(|value| serde_json::from_value::<Vec<PlatformQuota>>(value.clone()).ok())
        .filter(|quotas| !quotas.is_empty());
    let reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| Some("对话中生成".into()));
    let plan = plan::generate_plan(
        app,
        &state.db,
        &state.llm,
        GeneratePlanRequest {
            reason,
            focus_tags,
            count,
            platforms,
        },
    )
    .await?;
    let titles: Vec<Value> = plan
        .items
        .iter()
        .map(|i| json!({"problem_id": i.problem_id, "title": i.title, "kind": i.kind, "reason": i.reason}))
        .collect();
    Ok((
        format!("已生成 {} 道题的训练计划", plan.items.len()),
        json!({ "plan_id": plan.id, "rationale": plan.rationale, "items": titles }),
    ))
}

async fn sync_platform_tool(
    state: &SharedState,
    app: &AppHandle,
    args: &Value,
) -> AppResult<(String, Value)> {
    let platform = args
        .get("platform")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::invalid("缺少 platform"))?;
    if !crate::platforms::PLATFORMS.contains(&platform) {
        return Err(AppError::invalid("不支持的平台"));
    }
    crate::platforms::sync_platform(app, &state.db, &state.automation, platform).await?;
    Ok((
        format!("{} 同步完成", crate::platforms::platform_name(platform)),
        json!({ "platform": platform, "ok": true }),
    ))
}

async fn run_samples_tool(state: &SharedState, args: &Value) -> AppResult<(String, Value)> {
    let problem_id = args
        .get("problem_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::invalid("缺少 problem_id"))?;
    let code = match args.get("code").and_then(|v| v.as_str()) {
        Some(code) if !code.trim().is_empty() => code.to_string(),
        _ => judge::get_draft(&state.db, problem_id)
            .await
            .map(|(_, code)| code)
            .ok_or_else(|| AppError::invalid("这道题还没有草稿，请在编辑器中写代码后重试"))?,
    };
    let result = judge::run_samples(
        &state.db,
        RunSamplesRequest {
            problem_id: problem_id.to_string(),
            code,
            compiler_path: None,
            standard: None,
            save_draft: Some(false),
        },
    )
    .await?;
    let passed = result.samples.iter().filter(|s| s.status == "OK").count();
    let summary = if result.compile_ok {
        format!("样例 {passed}/{} 通过", result.samples.len())
    } else {
        "编译失败".into()
    };
    Ok((
        summary,
        json!({
            "compile_ok": result.compile_ok,
            "compile_output": truncate_chars(&result.compile_output, 3_000),
            "samples": result.samples.iter().map(|s| json!({
                "index": s.index, "status": s.status, "time_ms": s.time_ms, "diff": s.diff
            })).collect::<Vec<_>>()
        }),
    ))
}

async fn submit_draft_tool(
    state: &SharedState,
    app: &AppHandle,
    args: &Value,
) -> AppResult<(String, Value)> {
    let problem_id = args
        .get("problem_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::invalid("缺少 problem_id"))?;
    let (_, code) = judge::get_draft(&state.db, problem_id)
        .await
        .ok_or_else(|| AppError::invalid("还没有草稿，请先在题目页写代码"))?;
    let result = submission::submit(
        app,
        &state.db,
        &state.automation,
        &state.llm,
        SubmitRequest {
            problem_id: problem_id.to_string(),
            code,
            language: None,
            auto_submit: Some(false),
        },
    )
    .await?;
    Ok((
        result.message.clone(),
        json!({
            "needs_human": result.needs_human,
            "message": result.message,
            "automation_status": result.automation_status
        }),
    ))
}

async fn generate_report_tool(
    state: &SharedState,
    app: &AppHandle,
    args: &Value,
) -> AppResult<(String, Value)> {
    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("weekly");
    let report = reports::generate_report(
        app,
        &state.db,
        &state.llm,
        GenerateReportRequest {
            kind: kind.to_string(),
            period_start: None,
            period_end: None,
        },
    )
    .await?;
    Ok((
        format!("报告已生成：{}", report.title),
        json!({ "report_id": report.id, "title": report.title }),
    ))
}
