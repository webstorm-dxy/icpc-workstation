use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::llm::ProviderConfig;
use crate::models::RoleModelDto;
use crate::secrets;
use serde_json::Value;
use sqlx::Row;

pub const ROLES: [&str; 4] = ["chat", "recommend", "analyze", "report"];

pub fn default_base(kind: &str) -> &'static str {
    match kind {
        "opencodego" => "https://opencode.ai/inference/go",
        "deepseek" => "https://api.deepseek.com",
        "openai" => "https://api.openai.com/v1",
        "anthropic" => "https://api.anthropic.com/v1",
        _ => "",
    }
}

pub fn default_model(kind: &str) -> &'static str {
    match kind {
        "opencodego" => "deepseek-v4.1-flash",
        "deepseek" => "deepseek-flash",
        _ => "",
    }
}

pub fn provider_label(kind: &str) -> &'static str {
    match kind {
        "opencodego" => "OpenCode Go",
        "deepseek" => "DeepSeek",
        "openai" => "OpenAI",
        "anthropic" => "Anthropic",
        _ => "自定义 OpenAI 兼容",
    }
}

pub fn normalize_base(kind: &str, base: &str) -> String {
    let mut base = base.trim().trim_end_matches('/').to_string();
    if kind == "opencodego" && base.ends_with("/v1") {
        base.truncate(base.len() - 3);
        base = base.trim_end_matches('/').to_string();
    }
    base
}

pub async fn list_role_models(db: &Db) -> AppResult<Vec<RoleModelDto>> {
    let mut out = Vec::new();
    for role in ROLES {
        let row = sqlx::query(
            "SELECT rm.provider_id, rm.model_id, p.name AS provider_name, p.kind
             FROM role_models rm LEFT JOIN providers p ON p.id = rm.provider_id
             WHERE rm.role = ?1",
        )
        .bind(role)
        .fetch_optional(&db.pool)
        .await?;
        out.push(match row {
            Some(row) => RoleModelDto {
                role: role.to_string(),
                provider_id: row.get("provider_id"),
                model_id: row.get("model_id"),
                provider_name: row.get("provider_name"),
                provider_kind: row.get("kind"),
            },
            None => RoleModelDto {
                role: role.to_string(),
                provider_id: None,
                model_id: None,
                provider_name: None,
                provider_kind: None,
            },
        });
    }
    Ok(out)
}

pub struct ResolvedProvider {
    pub provider_id: String,
    pub model: String,
    pub config: ProviderConfig,
}

pub async fn resolve_role(db: &Db, role: &str, session_id: &str) -> AppResult<ResolvedProvider> {
    let row = sqlx::query(
        "SELECT rm.provider_id, rm.model_id, p.kind, p.name, p.base_url, p.config_json, p.enabled
         FROM role_models rm JOIN providers p ON p.id = rm.provider_id
         WHERE rm.role = ?1 AND p.enabled = 1",
    )
    .bind(role)
    .fetch_optional(&db.pool)
    .await?;
    if let Some(row) = row {
        let provider_id: String = row.get("provider_id");
        let model: String = row.get("model_id");
        let kind: String = row.get("kind");
        let name: String = row.get("name");
        let base_url: String = normalize_base(&kind, row.get("base_url"));
        let api_key = secrets::get_secret(&provider_id)?.ok_or_else(|| {
            AppError::new(
                "E_PROVIDER_KEY",
                format!("{name} 的 API Key 不在系统钥匙串中，请到设置中重新填写"),
            )
        })?;
        return Ok(ResolvedProvider {
            provider_id: provider_id.clone(),
            model: model.clone(),
            config: ProviderConfig {
                id: provider_id,
                kind,
                base_url,
                api_key,
                session_id: session_id.to_string(),
            },
        });
    }
    // No role mapping: fall back to the first enabled provider and its default model.
    let row = sqlx::query(
        "SELECT id, kind, name, base_url, config_json FROM providers WHERE enabled = 1 ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(&db.pool)
    .await?;
    let Some(row) = row else {
        return Err(AppError::new(
            "E_PROVIDER_NONE",
            "还没有配置模型提供商，请先完成 Agent 配置",
        ));
    };
    let provider_id: String = row.get("id");
    let kind: String = row.get("kind");
    let name: String = row.get("name");
    let base_url: String = normalize_base(&kind, row.get("base_url"));
    let config: Value =
        serde_json::from_str(row.get::<&str, _>("config_json")).unwrap_or(Value::Null);
    let model = config
        .get("default_model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default_model(&kind).to_string());
    if model.is_empty() {
        return Err(AppError::new(
            "E_PROVIDER_MODEL",
            "请先在设置中为角色选择模型",
        ));
    }
    let api_key = secrets::get_secret(&provider_id)?.ok_or_else(|| {
        AppError::new(
            "E_PROVIDER_KEY",
            format!("{name} 的 API Key 不在系统钥匙串中，请到设置中重新填写"),
        )
    })?;
    Ok(ResolvedProvider {
        provider_id: provider_id.clone(),
        model,
        config: ProviderConfig {
            id: provider_id,
            kind,
            base_url,
            api_key,
            session_id: session_id.to_string(),
        },
    })
}

pub async fn resolve_by_id(
    db: &Db,
    provider_id: &str,
    session_id: &str,
) -> AppResult<ResolvedProvider> {
    let row =
        sqlx::query("SELECT id, kind, name, base_url, config_json FROM providers WHERE id = ?1")
            .bind(provider_id)
            .fetch_optional(&db.pool)
            .await?
            .ok_or_else(|| AppError::not_found("模型提供商"))?;
    let id: String = row.get("id");
    let kind: String = row.get("kind");
    let name: String = row.get("name");
    let base_url: String = normalize_base(&kind, row.get("base_url"));
    let config: Value =
        serde_json::from_str(row.get::<&str, _>("config_json")).unwrap_or(Value::Null);
    let model = config
        .get("default_model")
        .and_then(|v| v.as_str())
        .unwrap_or(default_model(&kind))
        .to_string();
    let api_key = secrets::get_secret(&id)?
        .ok_or_else(|| AppError::new("E_PROVIDER_KEY", format!("{name} 还没有保存 API Key")))?;
    Ok(ResolvedProvider {
        provider_id: id.clone(),
        model,
        config: ProviderConfig {
            id,
            kind,
            base_url,
            api_key,
            session_id: session_id.to_string(),
        },
    })
}

pub fn session_for(prefix: &str) -> String {
    format!("ses_{}{}", prefix, uuid::Uuid::now_v7().simple())
}
