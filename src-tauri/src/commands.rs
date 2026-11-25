use crate::agent;
use crate::analysis;
use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::judge;
use crate::models::*;
use crate::plan;
use crate::platforms;
use crate::provider;
use crate::reports;
use crate::secrets;
use crate::state::SharedState;
use crate::submission;
use crate::util::now_iso;
use chrono::{Duration as ChronoDuration, NaiveDate, Utc};
use serde_json::{Value, json};
use sqlx::Row;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

fn known_platform(platform: &str) -> AppResult<()> {
    if platforms::PLATFORMS.contains(&platform) {
        Ok(())
    } else {
        Err(AppError::invalid(format!("不支持的平台: {platform}")))
    }
}

fn default_account(platform: &str) -> platforms::Account {
    platforms::Account {
        platform: platform.to_string(),
        handle: None,
        uid: None,
        logged_in: false,
        rating: 0,
        max_rating: 0,
        solved_count: 0,
        meta: json!({}),
        last_synced_at: None,
    }
}

// ---------------------------------------------------------------------------
// Onboarding & settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_onboarding_state(state: State<'_, SharedState>) -> AppResult<OnboardingStateDto> {
    let complete = state
        .db
        .get_setting("onboarding.complete")
        .await
        .map(|v| v == "true")
        .unwrap_or(false);
    let step = state
        .db
        .get_setting("onboarding.step")
        .await
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let configured_platforms = sqlx::query(
        "SELECT platform FROM platform_accounts WHERE COALESCE(handle,'') != '' OR COALESCE(uid,'') != '' OR logged_in = 1",
    )
    .fetch_all(&state.db.pool)
    .await?
    .into_iter()
    .map(|row| row.get::<String, _>("platform"))
    .collect::<Vec<_>>();
    let provider_ready: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers")
        .fetch_one(&state.db.pool)
        .await
        .unwrap_or(0);
    let selected: Vec<String> = state
        .db
        .get_json::<Vec<String>>("onboarding.platforms")
        .await
        .unwrap_or_default();
    Ok(OnboardingStateDto {
        complete,
        step,
        provider_ready: provider_ready > 0,
        local_only: false,
        platforms: selected,
        configured_platforms,
    })
}

#[tauri::command]
pub async fn set_onboarding_step(state: State<'_, SharedState>, step: i64) -> AppResult<()> {
    state
        .db
        .set_setting("onboarding.step", step.to_string())
        .await
}

#[tauri::command]
pub async fn set_onboarding_platforms(
    state: State<'_, SharedState>,
    platforms: Vec<String>,
) -> AppResult<()> {
    state.db.set_json("onboarding.platforms", &platforms).await
}

#[tauri::command]
pub async fn complete_onboarding(state: State<'_, SharedState>) -> AppResult<OnboardingStateDto> {
    state.db.set_setting("onboarding.complete", "true").await?;
    state.db.set_setting("onboarding.step", "6").await?;
    // If the user hasn't chosen role models, use the first provider's default.
    let role_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM role_models")
        .fetch_one(&state.db.pool)
        .await
        .unwrap_or(0);
    if role_count == 0 {
        if let Some(row) = sqlx::query(
            "SELECT id, kind, config_json FROM providers WHERE enabled = 1 ORDER BY created_at LIMIT 1",
        )
        .fetch_optional(&state.db.pool)
        .await?
        {
            let provider_id: String = row.get("id");
            let kind: String = row.get("kind");
            let config: Value =
                serde_json::from_str(row.get::<&str, _>("config_json")).unwrap_or(Value::Null);
            let model = config
                .get("default_model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| provider::default_model(&kind).to_string());
            if !model.is_empty() {
                for role in provider::ROLES {
                    let _ = sqlx::query(
                        "INSERT INTO role_models(role, provider_id, model_id, params_json)
                         VALUES(?1, ?2, ?3, '{}')
                         ON CONFLICT(role) DO UPDATE SET provider_id = excluded.provider_id, model_id = excluded.model_id",
                    )
                    .bind(role)
                    .bind(&provider_id)
                    .bind(&model)
                    .execute(&state.db.pool)
                    .await;
                }
            }
        }
    }
    get_onboarding_state(state).await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, SharedState>) -> AppResult<SettingsDto> {
    Ok(state
        .db
        .get_json::<SettingsDto>("settings")
        .await
        .unwrap_or_default())
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, SharedState>,
    patch: SettingsPatch,
) -> AppResult<SettingsDto> {
    let mut settings = state
        .db
        .get_json::<SettingsDto>("settings")
        .await
        .unwrap_or_default();
    if let Some(v) = patch.theme {
        settings.theme = if v == "light" {
            "light".into()
        } else {
            "dark".into()
        };
    }
    if let Some(v) = patch.default_language {
        settings.default_language = v;
    }
    if let Some(v) = patch.compiler_path {
        settings.compiler_path = v.filter(|s| !s.trim().is_empty());
    }
    if let Some(v) = patch.compiler_standard {
        settings.compiler_standard = v;
    }
    if let Some(v) = patch.auto_submit {
        settings.auto_submit = v;
    }
    if let Some(v) = patch.require_submit_confirm {
        settings.require_submit_confirm = v;
    }
    if let Some(v) = patch.allow_code_upload {
        settings.allow_code_upload = v;
    }
    if let Some(v) = patch.luogu_sync_depth {
        settings.luogu_sync_depth = v.clamp(50, 2000);
    }
    if let Some(v) = patch.auto_refresh_plan {
        settings.auto_refresh_plan = v;
    }
    if let Some(v) = patch.report_dir {
        settings.report_dir = v.filter(|s| !s.trim().is_empty());
    }
    if let Some(v) = patch.mock_mode {
        settings.mock_mode = v;
    }
    if let Some(v) = patch.lsp_enabled {
        settings.lsp_enabled = v;
    }
    if let Some(v) = patch.lsp_server_path {
        settings.lsp_server_path = v.filter(|s| !s.trim().is_empty());
    }
    state.db.set_json("settings", &settings).await?;
    Ok(settings)
}

#[tauri::command]
pub async fn reset_data(app: AppHandle, state: State<'_, SharedState>) -> AppResult<()> {
    let providers = sqlx::query("SELECT id FROM providers")
        .fetch_all(&state.db.pool)
        .await
        .unwrap_or_default();
    for row in providers {
        let id: String = row.get("id");
        let _ = secrets::delete_secret(&id);
    }
    state.db.reset_user_data().await?;
    state
        .db
        .set_json("settings", &SettingsDto::default())
        .await?;
    let _ = app.emit("data://reset", json!({}));
    Ok(())
}

#[tauri::command]
pub async fn get_app_info(app: AppHandle, state: State<'_, SharedState>) -> AppResult<Value> {
    Ok(json!({
        "version": app.package_info().version.to_string(),
        "dataDir": state.data_dir.to_string_lossy(),
        "model": "deepseek-v4.1-flash",
    }))
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

fn provider_dto_from_row(row: &sqlx::sqlite::SqliteRow) -> ProviderDto {
    let id: String = row.get("id");
    ProviderDto {
        id: id.clone(),
        kind: row.get("kind"),
        name: row.get("name"),
        base_url: row.get("base_url"),
        has_key: secrets::get_secret(&id).ok().flatten().is_some(),
        enabled: row.get::<i64, _>("enabled") != 0,
        config: serde_json::from_str(row.get::<&str, _>("config_json")).unwrap_or(json!({})),
    }
}

#[tauri::command]
pub async fn list_providers(state: State<'_, SharedState>) -> AppResult<Vec<ProviderDto>> {
    let rows = sqlx::query("SELECT * FROM providers ORDER BY created_at ASC")
        .fetch_all(&state.db.pool)
        .await?;
    Ok(rows.iter().map(provider_dto_from_row).collect())
}

#[tauri::command]
pub async fn upsert_provider(
    state: State<'_, SharedState>,
    input: ProviderUpsert,
) -> AppResult<ProviderDto> {
    if input.kind.trim().is_empty() {
        return Err(AppError::invalid("缺少提供商类型"));
    }
    let existing = if let Some(id) = &input.id {
        sqlx::query("SELECT * FROM providers WHERE id = ?1")
            .bind(id)
            .fetch_optional(&state.db.pool)
            .await?
    } else {
        None
    };
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    let base = provider::normalize_base(&input.kind, &input.base_url);
    let base = if base.is_empty() {
        provider::default_base(&input.kind).to_string()
    } else {
        base
    };
    if base.is_empty() {
        return Err(AppError::invalid("请填写 Base URL"));
    }
    let name = if input.name.trim().is_empty() {
        provider::provider_label(&input.kind).to_string()
    } else {
        input.name.trim().to_string()
    };
    let existing_config: Value = existing
        .as_ref()
        .map(|row| serde_json::from_str(row.get::<&str, _>("config_json")).unwrap_or(json!({})))
        .unwrap_or_else(|| json!({}));
    let mut config = input
        .config
        .clone()
        .filter(|v| v.is_object())
        .unwrap_or(existing_config.clone());
    if config
        .get("default_model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty()
    {
        let fallback = existing_config
            .get("default_model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| provider::default_model(&input.kind).to_string());
        if !fallback.is_empty() {
            config["default_model"] = json!(fallback);
        }
    }
    if let Some(key) = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        secrets::set_secret(&id, key)?;
    }
    sqlx::query(
        "INSERT INTO providers(id, kind, name, base_url, secret_ref, enabled, config_json, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
           kind = excluded.kind, name = excluded.name, base_url = excluded.base_url,
           secret_ref = excluded.secret_ref, config_json = excluded.config_json",
    )
    .bind(&id)
    .bind(&input.kind)
    .bind(&name)
    .bind(&base)
    .bind(&id)
    .bind(config.to_string())
    .bind(now_iso())
    .execute(&state.db.pool)
    .await?;
    // First provider: make all four roles usable immediately.
    let provider_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM providers")
        .fetch_one(&state.db.pool)
        .await
        .unwrap_or(0);
    let role_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM role_models")
        .fetch_one(&state.db.pool)
        .await
        .unwrap_or(0);
    if provider_count == 1 && role_count == 0 {
        let model = config
            .get("default_model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !model.is_empty() {
            for role in provider::ROLES {
                let _ = sqlx::query(
                    "INSERT INTO role_models(role, provider_id, model_id, params_json)
                     VALUES(?1, ?2, ?3, '{}')
                     ON CONFLICT(role) DO UPDATE SET provider_id = excluded.provider_id, model_id = excluded.model_id",
                )
                .bind(role)
                .bind(&id)
                .bind(&model)
                .execute(&state.db.pool)
                .await;
            }
        }
    }
    let row = sqlx::query("SELECT * FROM providers WHERE id = ?1")
        .bind(&id)
        .fetch_one(&state.db.pool)
        .await?;
    Ok(provider_dto_from_row(&row))
}

#[tauri::command]
pub async fn delete_provider(state: State<'_, SharedState>, id: String) -> AppResult<()> {
    let _ = secrets::delete_secret(&id);
    sqlx::query("DELETE FROM role_models WHERE provider_id = ?1")
        .bind(&id)
        .execute(&state.db.pool)
        .await?;
    sqlx::query("DELETE FROM providers WHERE id = ?1")
        .bind(&id)
        .execute(&state.db.pool)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn test_provider(
    state: State<'_, SharedState>,
    input: ProviderUpsert,
) -> AppResult<ProviderTestResult> {
    let api_key = match input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(key) => key.to_string(),
        None => input
            .id
            .as_deref()
            .and_then(|id| secrets::get_secret(id).ok().flatten())
            .unwrap_or_default(),
    };
    if api_key.is_empty() {
        return Ok(ProviderTestResult {
            ok: false,
            models: vec![],
            message: "请先填写 API Key".into(),
        });
    }
    let base = provider::normalize_base(&input.kind, &input.base_url);
    let base = if base.is_empty() {
        provider::default_base(&input.kind).to_string()
    } else {
        base
    };
    let cfg = crate::llm::ProviderConfig {
        id: input.id.clone().unwrap_or_else(|| "test".into()),
        kind: input.kind.clone(),
        base_url: base,
        api_key,
        session_id: provider::session_for("test"),
    };
    Ok(state.llm.test(&cfg).await)
}

#[tauri::command]
pub async fn list_models(
    state: State<'_, SharedState>,
    provider_id: String,
) -> AppResult<Vec<ModelDto>> {
    let resolved =
        provider::resolve_by_id(&state.db, &provider_id, &provider::session_for("models")).await?;
    state.llm.list_models(&resolved.config).await
}

#[tauri::command]
pub async fn get_provider_usage(
    state: State<'_, SharedState>,
    provider_id: String,
) -> AppResult<ProviderUsageDto> {
    let resolved =
        provider::resolve_by_id(&state.db, &provider_id, &provider::session_for("usage")).await?;
    state.llm.usage(&resolved.config).await
}

#[tauri::command]
pub async fn get_role_models(state: State<'_, SharedState>) -> AppResult<Vec<RoleModelDto>> {
    provider::list_role_models(&state.db).await
}

#[tauri::command]
pub async fn set_role_model(
    state: State<'_, SharedState>,
    input: RoleModelUpsert,
) -> AppResult<()> {
    if !provider::ROLES.contains(&input.role.as_str()) {
        return Err(AppError::invalid("未知的 Agent 角色"));
    }
    sqlx::query(
        "INSERT INTO role_models(role, provider_id, model_id, params_json) VALUES(?1, ?2, ?3, '{}')
         ON CONFLICT(role) DO UPDATE SET provider_id = excluded.provider_id, model_id = excluded.model_id",
    )
    .bind(&input.role)
    .bind(&input.provider_id)
    .bind(&input.model_id)
    .execute(&state.db.pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Platforms, automation, sync
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_platforms(state: State<'_, SharedState>) -> AppResult<Vec<PlatformDto>> {
    let mut out = Vec::new();
    for platform in platforms::PLATFORMS {
        let account = platforms::load_account(&state.db, platform).await?;
        out.push(platforms::platform_dto(platform, account.as_ref()));
    }
    Ok(out)
}

#[tauri::command]
pub async fn save_platform_account(
    state: State<'_, SharedState>,
    platform: String,
    handle: String,
) -> AppResult<PlatformDto> {
    known_platform(&platform)?;
    let handle = handle.trim().to_string();
    let mut account = platforms::load_account(&state.db, &platform)
        .await?
        .unwrap_or_else(|| default_account(&platform));
    if platform == "luogu" {
        account.uid = if handle.is_empty() {
            None
        } else {
            Some(handle)
        };
    } else {
        account.handle = if handle.is_empty() {
            None
        } else {
            Some(handle)
        };
    }
    platforms::save_account(&state.db, &account).await?;
    let account = platforms::load_account(&state.db, &platform).await?;
    Ok(platforms::platform_dto(&platform, account.as_ref()))
}

#[tauri::command]
pub async fn open_platform_login(
    app: AppHandle,
    state: State<'_, SharedState>,
    platform: String,
    bounds: Option<AutomationBounds>,
) -> AppResult<AutomationStateDto> {
    known_platform(&platform)?;
    state
        .automation
        .open(&app, &platform, platforms::login_url(&platform), bounds)
        .await?;
    Ok(state.automation.state(&platform))
}

#[tauri::command]
pub async fn open_automation(
    app: AppHandle,
    state: State<'_, SharedState>,
    platform: String,
    url: String,
    bounds: Option<AutomationBounds>,
) -> AppResult<AutomationStateDto> {
    known_platform(&platform)?;
    state.automation.open(&app, &platform, &url, bounds).await?;
    Ok(state.automation.state(&platform))
}

#[tauri::command]
pub async fn check_platform_handle(platform: String, handle: String) -> AppResult<String> {
    known_platform(&platform)?;
    platforms::check_handle(&platform, &handle).await
}

#[tauri::command]
pub async fn verify_platform_login(
    app: AppHandle,
    state: State<'_, SharedState>,
    platform: String,
) -> AppResult<PlatformLoginResult> {
    known_platform(&platform)?;
    platforms::verify_login(&app, &state.db, &state.automation, &platform).await
}

#[tauri::command]
pub async fn set_automation_bounds(
    app: AppHandle,
    state: State<'_, SharedState>,
    platform: String,
    bounds: AutomationBounds,
) -> AppResult<()> {
    state.automation.set_bounds(&app, &platform, &bounds);
    Ok(())
}

#[tauri::command]
pub async fn set_automation_visible(
    state: State<'_, SharedState>,
    platform: String,
    visible: bool,
) -> AppResult<()> {
    state.automation.set_visible(&platform, visible);
    Ok(())
}

#[tauri::command]
pub async fn close_automation(state: State<'_, SharedState>, platform: String) -> AppResult<()> {
    state.automation.close(&platform);
    Ok(())
}

#[tauri::command]
pub async fn get_automation_state(
    state: State<'_, SharedState>,
    platform: String,
) -> AppResult<AutomationStateDto> {
    Ok(state.automation.state(&platform))
}

#[tauri::command]
pub async fn start_sync(
    app: AppHandle,
    state: State<'_, SharedState>,
    platform: String,
) -> AppResult<()> {
    known_platform(&platform)?;
    {
        let mut syncing = state.syncing.lock().expect("sync lock");
        if syncing.contains(&platform) {
            return Err(AppError::new("E_SYNC_BUSY", "该平台正在同步中").retryable());
        }
        syncing.insert(platform.clone());
    }
    let app_task = app.clone();
    let state_task = state.inner().clone();
    let platform_task = platform.clone();
    tokio::spawn(async move {
        let _ = app_task.emit(
            "sync://progress",
            SyncProgressDto {
                platform: platform_task.clone(),
                stage: "start".into(),
                message: format!("开始同步 {}…", platforms::platform_name(&platform_task)),
                percent: 0.0,
                status: "running".into(),
            },
        );
        let result = platforms::sync_platform(
            &app_task,
            &state_task.db,
            &state_task.automation,
            &platform_task,
        )
        .await;
        match result {
            Ok(()) => {
                // A sync can backfill dozens of historical submissions. Refresh
                // the plan so already-attempted problems drop out of the
                // recommendation instead of waiting for the next submission.
                crate::plan::spawn_refresh(
                    app_task.clone(),
                    state_task.db.clone(),
                    state_task.llm.clone(),
                    None,
                    None,
                );
                let _ = app_task.emit(
                    "sync://progress",
                    SyncProgressDto {
                        platform: platform_task.clone(),
                        stage: "done".into(),
                        message: format!("{} 同步完成", platforms::platform_name(&platform_task)),
                        percent: 100.0,
                        status: "done".into(),
                    },
                );
            }
            Err(err) => {
                let _ = app_task.emit(
                    "sync://progress",
                    SyncProgressDto {
                        platform: platform_task.clone(),
                        stage: "error".into(),
                        message: err.message.clone(),
                        percent: 0.0,
                        status: "error".into(),
                    },
                );
                tracing::warn!("sync {} failed: {}", platform_task, err);
            }
        }
        state_task
            .syncing
            .lock()
            .expect("sync lock")
            .remove(&platform_task);
    });
    Ok(())
}

#[tauri::command]
pub async fn get_dashboard(state: State<'_, SharedState>) -> AppResult<DashboardDto> {
    let account_rows = sqlx::query("SELECT * FROM platform_accounts ORDER BY platform")
        .fetch_all(&state.db.pool)
        .await?;
    let mut accounts = Vec::new();
    let mut rating_points = Vec::new();
    for row in account_rows {
        let platform: String = row.get("platform");
        let meta: Value =
            serde_json::from_str(row.get::<&str, _>("meta_json")).unwrap_or(json!({}));
        if let Some(history) = meta.get("history").and_then(|v| v.as_array()) {
            for point in history {
                if let (Some(at), Some(rating)) = (
                    point.get("at").and_then(|v| v.as_str()),
                    point.get("rating").and_then(|v| v.as_i64()),
                ) {
                    rating_points.push(RatingPointDto {
                        at: at.to_string(),
                        rating,
                        platform: platform.clone(),
                    });
                }
            }
        }
        accounts.push(AccountCardDto {
            platform: platform.clone(),
            name: platforms::platform_name(&platform).to_string(),
            handle: row.get("handle"),
            rating: row.get::<Option<i64>, _>("rating").unwrap_or(0),
            max_rating: row.get::<Option<i64>, _>("max_rating").unwrap_or(0),
            solved_count: row.get::<Option<i64>, _>("solved_count").unwrap_or(0),
            last_synced_at: row.get("last_synced_at"),
            logged_in: row.get::<i64, _>("logged_in") != 0,
            meta,
        });
    }
    rating_points.sort_by(|a, b| a.at.cmp(&b.at));
    let tag_stats = analysis::skill_stats(&state.db).await?;
    let recent = sqlx::query(
        "SELECT s.id, s.platform, s.problem_id, p.title, s.verdict, s.language, s.submitted_at, s.analysis_status, s.time_ms
         FROM submissions s JOIN problems p ON p.id = s.problem_id
         ORDER BY s.submitted_at DESC LIMIT 10",
    )
    .fetch_all(&state.db.pool)
    .await?;
    let recent_submissions = recent
        .into_iter()
        .map(|row| SubmissionRowDto {
            id: row.get("id"),
            platform: row.get("platform"),
            problem_id: row.get("problem_id"),
            problem_title: row.get("title"),
            verdict: row.get("verdict"),
            language: row.get("language"),
            submitted_at: row.get("submitted_at"),
            analysis_status: row.get("analysis_status"),
            time_ms: row.get("time_ms"),
        })
        .collect();
    let analysis_rows = sqlx::query(
        "SELECT a.submission_id, s.problem_id, p.title, s.verdict, a.payload_json, a.created_at
         FROM analyses a JOIN submissions s ON s.id = a.submission_id
         JOIN problems p ON p.id = s.problem_id ORDER BY a.created_at DESC LIMIT 8",
    )
    .fetch_all(&state.db.pool)
    .await?;
    let analyses = analysis_rows
        .into_iter()
        .map(|row| {
            let payload: Value =
                serde_json::from_str(row.get::<&str, _>("payload_json")).unwrap_or(json!({}));
            AnalysisFeedDto {
                submission_id: row.get("submission_id"),
                problem_id: row.get("problem_id"),
                problem_title: row.get("title"),
                verdict: row.get("verdict"),
                summary: payload
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                category: payload
                    .get("failure_category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                created_at: row.get("created_at"),
            }
        })
        .collect();
    let active_plan = plan::active_plan(&state.db).await?;
    let totals = totals(&state.db).await?;
    Ok(DashboardDto {
        accounts,
        rating_points,
        tag_stats,
        recent_submissions,
        analyses,
        active_plan,
        totals,
    })
}

async fn totals(db: &Db) -> AppResult<TotalsDto> {
    let submissions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM submissions")
        .fetch_one(&db.pool)
        .await
        .unwrap_or(0);
    let accepted: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM submissions WHERE verdict = 'AC'")
        .fetch_one(&db.pool)
        .await
        .unwrap_or(0);
    let solved: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT problem_id) FROM submissions WHERE verdict = 'AC'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap_or(0);
    let dates: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT substr(submitted_at, 1, 10) AS d FROM submissions ORDER BY d DESC LIMIT 120",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap_or_default();
    let mut streak = 0i64;
    let today = Utc::now().date_naive();
    let mut cursor = today;
    let mut first = true;
    for date in dates {
        if let Ok(parsed) = NaiveDate::parse_from_str(&date, "%Y-%m-%d") {
            if first && parsed < today - ChronoDuration::days(1) {
                break;
            }
            first = false;
            if parsed == cursor || (cursor == today && parsed == today - ChronoDuration::days(1)) {
                streak += 1;
                cursor = parsed - ChronoDuration::days(1);
            } else if parsed < cursor {
                break;
            }
        }
    }
    Ok(TotalsDto {
        solved,
        submissions,
        accepted,
        acceptance_rate: if submissions > 0 {
            accepted as f64 / submissions as f64
        } else {
            0.0
        },
        streak_days: streak,
    })
}

// ---------------------------------------------------------------------------
// Problems, drafts, judge
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_problem(
    app: AppHandle,
    state: State<'_, SharedState>,
    problem_id: String,
    refresh: Option<bool>,
) -> AppResult<ProblemDto> {
    // `refresh` backs the "重新获取" button: it re-parses the statement even
    // when one is already stored.
    platforms::fetch_problem(
        &app,
        &state.db,
        &state.automation,
        &problem_id,
        refresh.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn get_draft(
    state: State<'_, SharedState>,
    problem_id: String,
) -> AppResult<Option<DraftDto>> {
    Ok(judge::get_draft(&state.db, &problem_id)
        .await
        .map(|(language, code)| DraftDto { language, code }))
}

#[tauri::command]
pub async fn save_draft(
    state: State<'_, SharedState>,
    problem_id: String,
    code: String,
    language: Option<String>,
) -> AppResult<()> {
    judge::save_draft_code(
        &state.db,
        &problem_id,
        &code,
        language.as_deref().unwrap_or("cpp"),
    )
    .await
}

#[tauri::command]
pub async fn run_samples(
    state: State<'_, SharedState>,
    req: RunSamplesRequest,
) -> AppResult<JudgeResultDto> {
    judge::run_samples(&state.db, req).await
}

#[tauri::command]
pub async fn detect_toolchains() -> AppResult<Vec<ToolchainDto>> {
    Ok(judge::detect_toolchains().await)
}

// ---------------------------------------------------------------------------
// Language server (LSP)
//
// The protocol itself lives in the webview (`src/lib/lsp.ts`); these commands
// only move JSON-RPC payloads across the process boundary, so every one of
// them is a thin, ordered adapter over `LspManager`.
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn detect_lsp_servers(
    state: State<'_, SharedState>,
    refresh: Option<bool>,
) -> AppResult<Vec<LspServerDto>> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        shared.lsp.detect_servers(refresh.unwrap_or(false))
    })
    .await
    .map_err(AppError::internal)
}

#[tauri::command]
pub async fn start_lsp_session(
    app: AppHandle,
    state: State<'_, SharedState>,
    input: StartLspInput,
) -> AppResult<LspSessionDto> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || shared.lsp.start(&app, &shared.data_dir, input))
        .await
        .map_err(AppError::internal)?
}

#[tauri::command]
pub fn send_lsp_message(
    state: State<'_, SharedState>,
    session_id: String,
    message: String,
) -> AppResult<()> {
    state.lsp.send(&session_id, &message)
}

#[tauri::command]
pub async fn stop_lsp_session(
    state: State<'_, SharedState>,
    session_id: String,
) -> AppResult<bool> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || shared.lsp.stop(&session_id))
        .await
        .map_err(AppError::internal)
}

// ---------------------------------------------------------------------------
// Plans
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn generate_plan(
    app: AppHandle,
    state: State<'_, SharedState>,
    req: GeneratePlanRequest,
) -> AppResult<PlanDto> {
    let plan = plan::generate_plan(&app, &state.db, &state.llm, req).await?;
    let _ = app.emit("plan://updated", json!({ "plan": plan }));
    Ok(plan)
}

#[tauri::command]
pub async fn get_active_plan(state: State<'_, SharedState>) -> AppResult<Option<PlanDto>> {
    plan::active_plan(&state.db).await
}

#[tauri::command]
pub async fn set_plan_item_status(
    state: State<'_, SharedState>,
    item_id: String,
    status: String,
) -> AppResult<()> {
    if !["pending", "in_progress", "done", "skipped"].contains(&status.as_str()) {
        return Err(AppError::invalid("无效的计划状态"));
    }
    plan::set_item_status(&state.db, &item_id, &status).await
}

// ---------------------------------------------------------------------------
// Submissions & analysis
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn submit_solution(
    app: AppHandle,
    state: State<'_, SharedState>,
    req: SubmitRequest,
) -> AppResult<SubmitResultDto> {
    submission::submit(&app, &state.db, &state.automation, &state.llm, req).await
}

#[tauri::command]
pub async fn capture_verdict(
    app: AppHandle,
    state: State<'_, SharedState>,
    req: CaptureVerdictRequest,
) -> AppResult<SubmitResultDto> {
    submission::capture_verdict(&app, &state.db, &state.automation, &state.llm, req).await
}

#[tauri::command]
pub async fn record_manual_submission(
    app: AppHandle,
    state: State<'_, SharedState>,
    req: ManualSubmissionRequest,
) -> AppResult<SubmissionDto> {
    submission::record_manual(&app, &state.db, &state.llm, req).await
}

#[tauri::command]
pub async fn get_submission(
    state: State<'_, SharedState>,
    submission_id: String,
) -> AppResult<SubmissionDto> {
    submission::submission_dto(&state.db, &submission_id).await
}

#[tauri::command]
pub async fn list_submissions(
    state: State<'_, SharedState>,
    limit: Option<i64>,
) -> AppResult<Vec<SubmissionRowDto>> {
    let rows = sqlx::query(
        "SELECT s.id, s.platform, s.problem_id, p.title, s.verdict, s.language, s.submitted_at, s.analysis_status, s.time_ms
         FROM submissions s JOIN problems p ON p.id = s.problem_id
         ORDER BY s.submitted_at DESC LIMIT ?1",
    )
    .bind(limit.unwrap_or(50).clamp(1, 500))
    .fetch_all(&state.db.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| SubmissionRowDto {
            id: row.get("id"),
            platform: row.get("platform"),
            problem_id: row.get("problem_id"),
            problem_title: row.get("title"),
            verdict: row.get("verdict"),
            language: row.get("language"),
            submitted_at: row.get("submitted_at"),
            analysis_status: row.get("analysis_status"),
            time_ms: row.get("time_ms"),
        })
        .collect())
}

#[tauri::command]
pub async fn get_analysis(
    state: State<'_, SharedState>,
    submission_id: String,
) -> AppResult<Option<AnalysisDto>> {
    Ok(analysis::load_analysis(&state.db, &submission_id).await)
}

#[tauri::command]
pub async fn reanalyze_submission(
    app: AppHandle,
    state: State<'_, SharedState>,
    submission_id: String,
) -> AppResult<AnalysisDto> {
    let dto = analysis::analyze_submission(&app, &state.db, &state.llm, &submission_id, true)
        .await?
        .ok_or_else(|| AppError::new("E_ANALYSIS_NO_CODE", "这条提交没有保存代码，无法重新分析"))?;
    Ok(dto)
}

// ---------------------------------------------------------------------------
// Reports & export
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn generate_report(
    app: AppHandle,
    state: State<'_, SharedState>,
    req: GenerateReportRequest,
) -> AppResult<ReportDto> {
    reports::generate_report(&app, &state.db, &state.llm, req).await
}

#[tauri::command]
pub async fn list_reports(state: State<'_, SharedState>) -> AppResult<Vec<ReportSummaryDto>> {
    reports::list_reports(&state.db).await
}

#[tauri::command]
pub async fn get_report(state: State<'_, SharedState>, report_id: String) -> AppResult<ReportDto> {
    reports::get_report(&state.db, &report_id).await
}

#[tauri::command]
pub async fn delete_report(state: State<'_, SharedState>, report_id: String) -> AppResult<()> {
    reports::delete_report(&state.db, &report_id).await
}

#[tauri::command]
pub async fn export_report(
    app: AppHandle,
    state: State<'_, SharedState>,
    req: ExportReportRequest,
) -> AppResult<String> {
    if req.format == "pdf" {
        *state.print_report_id.lock().expect("print lock") = Some(req.report_id.clone());
        if let Some(existing) = app.get_webview_window("print") {
            let _ = existing.close();
        }
        let _window =
            WebviewWindowBuilder::new(&app, "print", WebviewUrl::App("index.html".into()))
                .title("ICPC 训练报告")
                .inner_size(1000.0, 820.0)
                .build()
                .map_err(|e| AppError::new("E_REPORT_PRINT", format!("无法打开打印窗口: {e}")))?;
        let app_task = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
            if let Some(window) = app_task.get_webview_window("print") {
                let _ = window.eval("window.location.hash = '#/print/report';");
            }
            tokio::time::sleep(std::time::Duration::from_millis(1_800)).await;
            if let Some(window) = app_task.get_webview_window("print") {
                #[cfg(target_os = "macos")]
                {
                    let _ = window.print();
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = window.eval("window.print();");
                }
            }
        });
        return Ok(String::new());
    }
    let report = reports::get_report(&state.db, &req.report_id).await?;
    let content = if req.format == "html" {
        req.content
            .clone()
            .ok_or_else(|| AppError::invalid("缺少 HTML 内容"))?
    } else {
        report.content_md
    };
    let path = req
        .path
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| AppError::invalid("请选择导出路径"))?;
    std::fs::write(&path, content)?;
    Ok(path)
}

#[tauri::command]
pub async fn get_print_report_id(state: State<'_, SharedState>) -> AppResult<Option<String>> {
    Ok(state.print_report_id.lock().expect("print lock").clone())
}

#[tauri::command]
pub async fn print_report_window(app: AppHandle) -> AppResult<()> {
    if let Some(window) = app.get_webview_window("print") {
        #[cfg(target_os = "macos")]
        {
            let _ = window.print();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = window.eval("window.print();");
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn close_print_window(app: AppHandle) -> AppResult<()> {
    if let Some(window) = app.get_webview_window("print") {
        let _ = window.close();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Chat
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_conversations(state: State<'_, SharedState>) -> AppResult<Vec<ConversationDto>> {
    agent::list_conversations(&state.db).await
}

#[tauri::command]
pub async fn create_conversation(state: State<'_, SharedState>) -> AppResult<ConversationDto> {
    agent::create_conversation(&state.db).await
}

#[tauri::command]
pub async fn delete_conversation(state: State<'_, SharedState>, id: String) -> AppResult<()> {
    agent::delete_conversation(&state.db, &id).await
}

#[tauri::command]
pub async fn get_messages(
    state: State<'_, SharedState>,
    conversation_id: String,
) -> AppResult<Vec<ChatMessageDto>> {
    agent::get_messages(&state.db, &conversation_id).await
}

#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    state: State<'_, SharedState>,
    conversation_id: String,
    text: String,
) -> AppResult<SendMessageResult> {
    agent::send_message(state.inner().clone(), app, conversation_id, text).await
}

#[tauri::command]
pub async fn cancel_message(state: State<'_, SharedState>, message_id: String) -> AppResult<bool> {
    Ok(state.agent.cancel(&message_id))
}

#[tauri::command]
pub async fn confirm_tool(
    state: State<'_, SharedState>,
    request_id: String,
    approved: bool,
) -> AppResult<bool> {
    Ok(state.agent.resolve_confirmation(&request_id, approved))
}

// ---------------------------------------------------------------------------
// Demo data
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn seed_demo_data(app: AppHandle, state: State<'_, SharedState>) -> AppResult<String> {
    seed_demo(&app, state.inner()).await
}

pub async fn seed_demo(app: &AppHandle, state: &SharedState) -> AppResult<String> {
    let problems: Vec<(&str, &str, &str, i64, &str, &str)> = vec![
        (
            "4A",
            "Watermelon",
            "判断整数能否拆成两个正偶数",
            800,
            "implementation,math",
            "8\n",
        ),
        (
            "71A",
            "Way Too Long Words",
            "缩写过长的单词",
            800,
            "implementation,strings",
            "4\nword\nlocalization\ninternationalization\n",
        ),
        (
            "158A",
            "Next Round",
            "统计达到晋级分数的人数",
            800,
            "implementation",
            "8 5\n10 9 8 7 7 7 5 5\n",
        ),
        (
            "231A",
            "Team",
            "统计能解题的队伍数",
            800,
            "greedy,implementation",
            "3\n1 1 0\n1 1 1\n1 0 0\n",
        ),
        (
            "50A",
            "Domino piling",
            "多米诺骨牌最多覆盖多少格",
            800,
            "math,greedy",
            "2 4\n",
        ),
        (
            "282A",
            "Bit++",
            "解析 Bit++ 语句",
            800,
            "implementation",
            "1\n++X\n",
        ),
        (
            "1A",
            "Theatre Square",
            "铺满广场最少需要多少块石板",
            1200,
            "math",
            "6 6 4\n",
        ),
        (
            "112A",
            "Petya and Strings",
            "忽略大小写比较字符串",
            800,
            "implementation,strings",
            "aaaa\nbbbb\n",
        ),
        (
            "263A",
            "Beautiful Matrix",
            "把 1 移到矩阵中心的最少步数",
            800,
            "implementation",
            "",
        ),
        (
            "339A",
            "Helpful Maths",
            "对加法表达式排序",
            800,
            "greedy,sorting",
            "3+2+1\n",
        ),
        (
            "266A",
            "Stones on the Table",
            "最少拿走多少石子使相邻不同色",
            800,
            "implementation",
            "3\nRRG\n",
        ),
        (
            "116A",
            "Tram",
            "统计电车最大载客量",
            800,
            "implementation",
            "4\n0 3\n2 5\n4 2\n4 0\n",
        ),
        (
            "4C",
            "Registration system",
            "为重复的用户名生成新名字",
            1300,
            "data-structures,implementation,hashing",
            "4\nabacaba\nacaba\nabacaba\nacab\n",
        ),
        (
            "489C",
            "Given Length and Sum of Digits",
            "求指定长度与数位和的最小/最大数",
            1400,
            "dp,greedy",
            "2 15\n",
        ),
        (
            "451B",
            "Sort the Array",
            "判断能否翻转一个子段使数组有序",
            1300,
            "implementation,sortings",
            "3\n3 2 1\n",
        ),
        (
            "550A",
            "Two Substrings",
            "判断是否同时含 AB 与 BA",
            1500,
            "strings,implementation",
            "ABA\n",
        ),
        (
            "580A",
            "Kefa and First Steps",
            "最长不下降子段",
            900,
            "dp,implementation",
            "6\n2 2 1 3 4 1\n",
        ),
        (
            "913B",
            "Christmas Spruce",
            "判断树是否满足圣诞树条件",
            1200,
            "trees,implementation",
            "4\n1 1 1\n",
        ),
        (
            "977B",
            "Two-gram",
            "出现次数最多的二元组",
            800,
            "strings,implementation",
            "7\nABACABA\n",
        ),
        (
            "1095A",
            "Repeating Cipher",
            "解码重复加密字符串",
            800,
            "strings,implementation",
            "7\nbaabbb\n",
        ),
        (
            "1363A",
            "Odd Selection",
            "能否选奇数个数使和为奇数",
            1200,
            "brute-force,math",
            "3\n3 3\n1 2 3\n",
        ),
    ];
    for (native, title, _summary, rating, tags, sample_input) in &problems {
        let tags_vec: Vec<String> = tags.split(',').map(|s| s.to_string()).collect();
        let skill_tags = platforms::skill_tags_for("codeforces", &tags_vec);
        let samples = if sample_input.is_empty() {
            vec![]
        } else {
            vec![SampleDto {
                input: sample_input.to_string(),
                output: "示例输出".into(),
            }]
        };
        sqlx::query(
            "INSERT OR REPLACE INTO problems(id, platform, native_id, title, url, rating, normalized_rating, tags_json, skill_tags_json, time_limit_ms, memory_limit_mb, statement_md, samples_json, fetched_at)
             VALUES(?1, 'codeforces', ?2, ?3, ?4, ?5, ?5, ?6, ?7, 2000, 256, ?8, ?9, ?10)",
        )
        .bind(format!("cf:{native}"))
        .bind(native)
        .bind(title)
        .bind(format!("https://codeforces.com/problemset/problem/{}/{native}", native.trim_end_matches(|c: char| c.is_ascii_alphabetic())))
        .bind(rating)
        .bind(serde_json::to_string(&tags_vec).unwrap())
        .bind(serde_json::to_string(&skill_tags).unwrap())
        .bind(format!("这是演示题面：{title}。\n\n> 这是示例数据，用于展示界面；接入真实平台后会显示完整题面。"))
        .bind(serde_json::to_string(&samples).unwrap())
        .bind(now_iso())
        .execute(&state.db.pool)
        .await?;
    }
    let history = [
        (1700000000i64, 1200i64),
        (1710000000, 1280),
        (1720000000, 1350),
        (1730000000, 1310),
        (1740000000, 1450),
    ];
    let meta = json!({
        "rank": "specialist",
        "history": history.iter().map(|(ts, r)| json!({
            "at": chrono::DateTime::<Utc>::from_timestamp(*ts, 0)
                .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                .unwrap_or_else(now_iso),
            "rating": r,
            "contest": "Demo Round"
        })).collect::<Vec<_>>()
    });
    sqlx::query(
        "INSERT OR REPLACE INTO platform_accounts(platform, handle, uid, display_name, rating, max_rating, solved_count, logged_in, meta_json, last_synced_at)
         VALUES('codeforces', 'demo_coder', NULL, 'Demo Coder', 1450, 1600, 12, 1, ?1, ?2)",
    )
    .bind(meta.to_string())
    .bind(now_iso())
    .execute(&state.db.pool)
    .await?;
    let now = Utc::now();
    let demo_subs = [
        ("4A", "AC", 800, Some("YES"), Some(31)),
        ("71A", "AC", 800, Some("w8rd"), Some(46)),
        ("158A", "AC", 800, Some("0"), Some(46)),
        ("231A", "AC", 800, Some("2"), Some(46)),
        ("50A", "AC", 800, Some("4"), Some(31)),
        ("282A", "WA", 800, Some("0"), Some(46)),
        ("1A", "AC", 1200, Some("4"), Some(46)),
        ("4C", "WA", 1300, Some("NO"), Some(62)),
        ("489C", "TLE", 1400, None, None),
        ("451B", "WA", 1300, Some("no"), Some(46)),
        ("550A", "AC", 1500, Some("YES"), Some(46)),
        ("580A", "AC", 900, Some("3"), Some(31)),
    ];
    for (idx, (native, verdict, _rating, output, time)) in demo_subs.iter().enumerate() {
        let submitted = now - ChronoDuration::hours((idx as i64) * 7 + 2);
        let id = format!("cf:demo{native}");
        sqlx::query(
            "INSERT OR REPLACE INTO submissions(id, platform, native_id, problem_id, verdict, language, time_ms, memory_kb, submitted_at, captured_via, raw_json, analysis_status)
             VALUES(?1, 'codeforces', ?2, ?3, ?4, 'GNU G++17', ?5, 12000, ?6, 'sync', '{}', 'pending')",
        )
        .bind(&id)
        .bind(format!("demo-{native}"))
        .bind(format!("cf:{native}"))
        .bind(verdict)
        .bind(time)
        .bind(submitted.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .execute(&state.db.pool)
        .await?;
        if let Some(code) = output {
            let source = format!(
                "#include <bits/stdc++.h>\nusing namespace std;\nint main(){{ ios::sync_with_stdio(false); cin.tie(nullptr); /* demo {native} */ cout << \"{code}\" << '\\n'; }}\n"
            );
            sqlx::query(
                "INSERT OR REPLACE INTO submission_code(submission_id, code_text) VALUES(?1, ?2)",
            )
            .bind(&id)
            .bind(&source)
            .execute(&state.db.pool)
            .await?;
        }
    }
    let skills = [
        ("implementation", 1380.0, 14, 9),
        ("dp", 1120.0, 5, 0),
        ("graphs", 1180.0, 6, 1),
        ("math", 1420.0, 9, 6),
        ("greedy", 1260.0, 8, 4),
        ("data-structures", 1060.0, 4, 0),
        ("strings", 1300.0, 7, 5),
        ("binary-search", 1210.0, 3, 1),
    ];
    for (tag, rating, attempts, solved) in skills {
        sqlx::query(
            "INSERT OR REPLACE INTO skill_profiles(tag, rating, attempts, solved, last_delta, updated_at)
             VALUES(?1, ?2, ?3, ?4, 0, ?5)",
        )
        .bind(tag)
        .bind(rating)
        .bind(attempts)
        .bind(solved)
        .bind(now_iso())
        .execute(&state.db.pool)
        .await?;
    }
    let analysis_payload = json!({
        "summary": "4C 的 WA 很可能来自哈希/重复计数在插入顺序上的边界处理。",
        "failure_category": "algorithm",
        "root_cause": "重复用户名第一次出现时没有把新名字加入集合，导致第二次重复时生成了与原串冲突的名称。",
        "bug_patterns": ["哈希集合更新时机", "边界样例"],
        "evidence": ["平台判定：WA", "示例 4C 的重复序列"],
        "complexity": {"time":"O(n log n)","space":"O(n)","expected":"O(n log n)","verdict":"ok"},
        "fix_steps": ["每次生成新名字后立刻把它写回集合", "用 n=1 与连续重复的样例自测"],
        "next_practice_tags": ["data-structures", "hashing", "implementation"],
        "learning_points": ["先维护不变量，再写分支"],
        "confidence": 0.72
    });
    sqlx::query(
        "INSERT OR REPLACE INTO analyses(id, submission_id, provider_id, model_id, prompt_version, payload_json, created_at)
         VALUES(?1, 'cf:demo4C', NULL, NULL, 'demo', ?2, ?3)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(analysis_payload.to_string())
    .bind(now_iso())
    .execute(&state.db.pool)
    .await?;
    sqlx::query("UPDATE submissions SET analysis_status = 'done' WHERE id = 'cf:demo4C'")
        .execute(&state.db.pool)
        .await?;

    let conversation_id = uuid::Uuid::now_v7().to_string();
    let now_str = now_iso();
    sqlx::query(
        "INSERT OR REPLACE INTO conversations(id, title, created_at, updated_at) VALUES(?1, '今天练什么？', ?2, ?2)",
    )
    .bind(&conversation_id)
    .bind(&now_str)
    .execute(&state.db.pool)
    .await?;
    sqlx::query(
        "INSERT INTO messages(id, conversation_id, role, content_md, tool_calls_json, created_at)
         VALUES(?1, ?2, 'user', '根据我最近的表现，今天练什么？', '[]', ?3)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(&conversation_id)
    .bind(&now_str)
    .execute(&state.db.pool)
    .await?;
    sqlx::query(
        "INSERT INTO messages(id, conversation_id, role, content_md, tool_calls_json, created_at)
         VALUES(?1, ?2, 'assistant', ?3, '[]', ?4)",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(&conversation_id)
    .bind("我从你的 Codeforces 数据里看到：**implementation** 比较稳，但 **dp** 和 **data-structures** 明显偏弱，最近 4C 的 WA 也指向集合更新时机。\n\n建议今天这样安排：\n1. 先复盘 4C，用最小反例确认哈希更新顺序。\n2. 做 2 道 1300-1500 的 DP 入门题，重点写状态定义。\n3. 最后做 1 道 1200 的 data-structures 题，训练不变量。\n\n你可以在「训练」页查看我生成的计划，点进题目后直接写代码、跑样例。")
    .bind(now_iso())
    .execute(&state.db.pool)
    .await?;

    state.db.set_setting("onboarding.complete", "true").await?;
    state.db.set_setting("onboarding.step", "6").await?;
    state
        .db
        .set_json("onboarding.platforms", &vec!["codeforces".to_string()])
        .await?;
    let _ = plan::generate_demo_plan(
        app,
        &state.db,
        &state.llm,
        GeneratePlanRequest {
            reason: Some("演示训练计划".into()),
            focus_tags: None,
            count: Some(6),
            platforms: None,
        },
    )
    .await;
    let _ = reports::generate_report(
        app,
        &state.db,
        &state.llm,
        GenerateReportRequest {
            kind: "weekly".into(),
            period_start: None,
            period_end: None,
        },
    )
    .await;
    Ok("演示数据已写入".into())
}
