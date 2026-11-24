use crate::error::{AppError, AppResult};
use crate::util::now_iso;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::time::Duration;

#[derive(Clone)]
pub struct Db {
    pub pool: SqlitePool,
}

impl Db {
    pub async fn open(path: &Path) -> AppResult<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(10));
        let pool = SqlitePoolOptions::new()
            .max_connections(6)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn get_setting(&self, key: &str) -> Option<String> {
        sqlx::query("SELECT value FROM app_settings WHERE key = ?1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten()
            .map(|row| row.get::<String, _>("value"))
    }

    pub async fn set_setting(&self, key: &str, value: impl Into<String>) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO app_settings(key, value, updated_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(value.into())
        .bind(now_iso())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_json<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let raw = self.get_setting(key).await?;
        serde_json::from_str(&raw).ok()
    }

    pub async fn set_json<T: Serialize>(&self, key: &str, value: &T) -> AppResult<()> {
        let raw = serde_json::to_string(value).map_err(AppError::internal)?;
        self.set_setting(key, raw).await
    }

    pub async fn reset_user_data(&self) -> AppResult<()> {
        for table in [
            "providers",
            "role_models",
            "platform_accounts",
            "sync_runs",
            "problems",
            "submissions",
            "submission_code",
            "analyses",
            "skill_profiles",
            "training_plans",
            "plan_items",
            "conversations",
            "messages",
            "reports",
            "run_results",
            "drafts",
            "audit_log",
            "platform_cache",
            "app_settings",
        ] {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }
}
