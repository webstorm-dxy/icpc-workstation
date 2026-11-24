use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDto {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub base_url: String,
    pub has_key: bool,
    pub enabled: bool,
    pub config: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUpsert {
    pub id: Option<String>,
    pub kind: String,
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub config: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDto {
    pub id: String,
    pub name: String,
    pub context: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestResult {
    pub ok: bool,
    pub models: Vec<ModelDto>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMeterDto {
    pub window: String,
    pub percent: f64,
    pub status: String,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageDto {
    pub meters: Vec<UsageMeterDto>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleModelDto {
    pub role: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub provider_name: Option<String>,
    pub provider_kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleModelUpsert {
    pub role: String,
    pub provider_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingStateDto {
    pub complete: bool,
    pub step: i64,
    pub provider_ready: bool,
    pub local_only: bool,
    pub platforms: Vec<String>,
    pub configured_platforms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub login_url: String,
    pub needs_handle: bool,
    pub handle_label: String,
    pub logged_in: bool,
    pub handle: Option<String>,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformLoginResult {
    pub logged_in: bool,
    pub handle: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCardDto {
    pub platform: String,
    pub name: String,
    pub handle: Option<String>,
    pub rating: i64,
    pub max_rating: i64,
    pub solved_count: i64,
    pub last_synced_at: Option<String>,
    pub logged_in: bool,
    pub meta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RatingPointDto {
    pub at: String,
    pub rating: i64,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagStatDto {
    pub tag: String,
    pub rating: f64,
    pub attempts: i64,
    pub solved: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionRowDto {
    pub id: String,
    pub platform: String,
    pub problem_id: String,
    pub problem_title: String,
    pub verdict: String,
    pub language: Option<String>,
    pub submitted_at: String,
    pub analysis_status: String,
    pub time_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisFeedDto {
    pub submission_id: String,
    pub problem_id: String,
    pub problem_title: String,
    pub verdict: String,
    pub summary: String,
    pub category: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItemDto {
    pub id: String,
    pub problem_id: String,
    pub title: String,
    pub platform: String,
    pub url: String,
    pub rating: Option<i64>,
    pub tags: Vec<String>,
    pub kind: String,
    pub reason: String,
    pub expected_minutes: Option<i64>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDto {
    pub id: String,
    pub status: String,
    pub rationale: String,
    pub created_at: String,
    pub items: Vec<PlanItemDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TotalsDto {
    pub solved: i64,
    pub submissions: i64,
    pub accepted: i64,
    pub acceptance_rate: f64,
    pub streak_days: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardDto {
    pub accounts: Vec<AccountCardDto>,
    pub rating_points: Vec<RatingPointDto>,
    pub tag_stats: Vec<TagStatDto>,
    pub recent_submissions: Vec<SubmissionRowDto>,
    pub analyses: Vec<AnalysisFeedDto>,
    pub active_plan: Option<PlanDto>,
    pub totals: TotalsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleDto {
    pub input: String,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemDto {
    pub id: String,
    pub platform: String,
    pub native_id: String,
    pub title: String,
    pub url: String,
    pub rating: Option<i64>,
    pub normalized_rating: Option<i64>,
    pub tags: Vec<String>,
    pub skill_tags: Vec<String>,
    pub time_limit_ms: Option<i64>,
    pub memory_limit_mb: Option<i64>,
    pub statement_md: String,
    pub samples: Vec<SampleDto>,
    pub solved: bool,
    pub attempted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratePlanRequest {
    pub reason: Option<String>,
    pub focus_tags: Option<Vec<String>>,
    pub count: Option<i64>,
    pub platforms: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainDto {
    pub path: String,
    pub kind: String,
    pub version: String,
    pub supports_bits: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftDto {
    pub language: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSamplesRequest {
    pub problem_id: String,
    pub code: String,
    pub compiler_path: Option<String>,
    pub standard: Option<String>,
    pub save_draft: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleResultDto {
    pub index: i64,
    pub status: String,
    pub time_ms: i64,
    pub stdout: String,
    pub stderr: String,
    pub expected: String,
    pub diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JudgeResultDto {
    pub ok: bool,
    pub compile_ok: bool,
    pub compile_output: String,
    pub samples: Vec<SampleResultDto>,
    pub total_ms: i64,
    pub compiler: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitRequest {
    pub problem_id: String,
    pub code: String,
    pub language: Option<String>,
    pub auto_submit: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualSubmissionRequest {
    pub problem_id: String,
    pub code: String,
    pub verdict: String,
    pub language: Option<String>,
    pub time_ms: Option<i64>,
    pub memory_kb: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisDto {
    pub submission_id: String,
    pub summary: String,
    pub failure_category: String,
    pub root_cause: String,
    pub bug_patterns: Vec<String>,
    pub evidence: Vec<String>,
    pub complexity: Value,
    pub fix_steps: Vec<String>,
    pub next_practice_tags: Vec<String>,
    pub learning_points: Vec<String>,
    pub confidence: f64,
    pub created_at: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionDto {
    pub id: String,
    pub platform: String,
    pub problem_id: String,
    pub problem_title: String,
    pub verdict: String,
    pub language: Option<String>,
    pub time_ms: Option<i64>,
    pub memory_kb: Option<i64>,
    pub submitted_at: String,
    pub analysis_status: String,
    pub needs_human: bool,
    pub message: Option<String>,
    pub analysis: Option<AnalysisDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureVerdictRequest {
    pub problem_id: String,
    pub code: String,
    pub language: Option<String>,
    pub since_epoch: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitResultDto {
    pub needs_human: bool,
    pub message: String,
    pub submission: Option<SubmissionDto>,
    pub automation_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// CSS viewport height reported by the UI webview. On macOS the native
    /// child WebView is positioned relative to the window frame while the UI
    /// webview viewport starts below the title bar; the difference is applied
    /// as a top offset so the page does not cover the automation header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport_height: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationStateDto {
    pub platform: String,
    pub visible: bool,
    pub url: Option<String>,
    pub status: String,
    pub message: String,
    pub needs_human: bool,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDto {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallDto {
    pub id: String,
    pub name: String,
    pub status: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessageDto {
    pub id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
    pub tool_calls: Vec<ToolCallDto>,
    pub usage: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMessageResult {
    pub conversation_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfirmationDto {
    pub request_id: String,
    pub tool: String,
    pub summary: String,
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSummaryDto {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportDto {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    pub content_md: String,
    pub data: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateReportRequest {
    pub kind: String,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReportRequest {
    pub report_id: String,
    pub format: String,
    pub content: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub theme: String,
    pub default_language: String,
    pub compiler_path: Option<String>,
    pub compiler_standard: String,
    pub auto_submit: bool,
    pub require_submit_confirm: bool,
    pub allow_code_upload: bool,
    pub luogu_sync_depth: i64,
    pub auto_refresh_plan: bool,
    pub report_dir: Option<String>,
    pub mock_mode: bool,
}

impl Default for SettingsDto {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            default_language: "cpp".into(),
            compiler_path: None,
            compiler_standard: "gnu++17".into(),
            auto_submit: false,
            require_submit_confirm: true,
            allow_code_upload: true,
            luogu_sync_depth: 300,
            auto_refresh_plan: true,
            report_dir: None,
            mock_mode: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub theme: Option<String>,
    pub default_language: Option<String>,
    pub compiler_path: Option<Option<String>>,
    pub compiler_standard: Option<String>,
    pub auto_submit: Option<bool>,
    pub require_submit_confirm: Option<bool>,
    pub allow_code_upload: Option<bool>,
    pub luogu_sync_depth: Option<i64>,
    pub auto_refresh_plan: Option<bool>,
    pub report_dir: Option<Option<String>>,
    pub mock_mode: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgressDto {
    pub platform: String,
    pub stage: String,
    pub message: String,
    pub percent: f64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDeltaEvent {
    pub conversation_id: String,
    pub message_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolEvent {
    pub conversation_id: String,
    pub message_id: String,
    pub tool: ToolCallDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDoneEvent {
    pub conversation_id: String,
    pub message_id: String,
    pub content: String,
    pub usage: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationEvent {
    pub platform: String,
    pub status: String,
    pub message: String,
    pub needs_human: bool,
    pub hint: Option<String>,
    pub url: Option<String>,
}
