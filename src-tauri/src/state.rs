use crate::agent::AgentService;
use crate::automation::AutomationManager;
use crate::db::Db;
use crate::error::AppResult;
use crate::llm::LlmClient;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Manager;
use tracing_appender::non_blocking::WorkerGuard;

pub struct AppState {
    pub db: Db,
    pub llm: LlmClient,
    pub automation: AutomationManager,
    pub agent: AgentService,
    pub print_report_id: Mutex<Option<String>>,
    pub syncing: Mutex<HashSet<String>>,
    pub data_dir: PathBuf,
}

pub type SharedState = Arc<AppState>;

static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

pub async fn init(app: &tauri::AppHandle) -> AppResult<SharedState> {
    let data_dir = match std::env::var("ICPC_DATA_DIR") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => app
            .path()
            .app_data_dir()
            .map_err(crate::error::AppError::internal)?,
    };
    std::fs::create_dir_all(&data_dir)?;
    init_logging(&data_dir);
    let db = Db::open(&data_dir.join("workstation.db")).await?;
    Ok(Arc::new(AppState {
        db,
        llm: LlmClient::new(),
        automation: AutomationManager::default(),
        agent: AgentService::default(),
        print_report_id: Mutex::new(None),
        syncing: Mutex::new(HashSet::new()),
        data_dir,
    }))
}

fn init_logging(data_dir: &std::path::Path) {
    let log_dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let appender = tracing_appender::rolling::daily(log_dir, "workstation.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _ = LOG_GUARD.set(guard);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,icpc_workstation_lib=debug"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init();
}
