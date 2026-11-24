import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AgentDeltaEvent,
  AgentDoneEvent,
  AgentToolEvent,
  Analysis,
  AppInfo,
  AutomationBounds,
  AutomationEvent,
  AutomationState,
  ChatMessage,
  Conversation,
  Dashboard,
  Draft,
  JudgeResult,
  ModelInfo,
  OnboardingState,
  Plan,
  Platform,
  PlatformLoginResult,
  Problem,
  Provider,
  ProviderTestResult,
  ProviderUsage,
  Report,
  ReportSummary,
  RoleModel,
  Sample,
  SendMessageResult,
  Settings,
  Submission,
  SubmissionRow,
  SubmitResult,
  SyncProgress,
  ToolConfirmation,
  Toolchain,
} from "../types";

export const api = {
  // onboarding / settings
  getOnboardingState: () => invoke<OnboardingState>("get_onboarding_state"),
  setOnboardingStep: (step: number) => invoke<void>("set_onboarding_step", { step }),
  setOnboardingPlatforms: (platforms: string[]) =>
    invoke<void>("set_onboarding_platforms", { platforms }),
  completeOnboarding: () => invoke<OnboardingState>("complete_onboarding"),
  getSettings: () => invoke<Settings>("get_settings"),
  updateSettings: (patch: Partial<Settings> & Record<string, unknown>) =>
    invoke<Settings>("update_settings", { patch }),
  resetData: () => invoke<void>("reset_data"),
  getAppInfo: () => invoke<AppInfo>("get_app_info"),

  // providers
  listProviders: () => invoke<Provider[]>("list_providers"),
  upsertProvider: (input: {
    id?: string;
    kind: string;
    name: string;
    baseUrl: string;
    apiKey?: string;
    config?: Record<string, unknown>;
  }) => invoke<Provider>("upsert_provider", { input }),
  deleteProvider: (id: string) => invoke<void>("delete_provider", { id }),
  testProvider: (input: {
    id?: string;
    kind: string;
    name?: string;
    baseUrl: string;
    apiKey?: string;
    config?: Record<string, unknown>;
  }) => invoke<ProviderTestResult>("test_provider", { input }),
  listModels: (providerId: string) => invoke<ModelInfo[]>("list_models", { providerId }),
  getProviderUsage: (providerId: string) =>
    invoke<ProviderUsage>("get_provider_usage", { providerId }),
  getRoleModels: () => invoke<RoleModel[]>("get_role_models"),
  setRoleModel: (input: { role: string; providerId: string; modelId: string }) =>
    invoke<void>("set_role_model", { input }),

  // platforms / automation
  listPlatforms: () => invoke<Platform[]>("list_platforms"),
  savePlatformAccount: (platform: string, handle: string) =>
    invoke<Platform>("save_platform_account", { platform, handle }),
  openPlatformLogin: (platform: string, bounds?: AutomationBounds) =>
    invoke<AutomationState>("open_platform_login", { platform, bounds }),
  openAutomation: (platform: string, url: string, bounds?: AutomationBounds) =>
    invoke<AutomationState>("open_automation", { platform, url, bounds }),
  verifyPlatformLogin: (platform: string) =>
    invoke<PlatformLoginResult>("verify_platform_login", { platform }),
  setAutomationBounds: (platform: string, bounds: AutomationBounds) =>
    invoke<void>("set_automation_bounds", { platform, bounds }),
  setAutomationVisible: (platform: string, visible: boolean) =>
    invoke<void>("set_automation_visible", { platform, visible }),
  closeAutomation: (platform: string) => invoke<void>("close_automation", { platform }),
  getAutomationState: (platform: string) =>
    invoke<AutomationState>("get_automation_state", { platform }),
  startSync: (platform: string) => invoke<void>("start_sync", { platform }),
  getDashboard: () => invoke<Dashboard>("get_dashboard"),

  // problems / judge
  getProblem: (problemId: string) => invoke<Problem>("get_problem", { problemId }),
  getDraft: (problemId: string) => invoke<Draft | null>("get_draft", { problemId }),
  saveDraft: (problemId: string, code: string, language = "cpp") =>
    invoke<void>("save_draft", { problemId, code, language }),
  runSamples: (req: {
    problemId: string;
    code: string;
    compilerPath?: string;
    standard?: string;
    saveDraft?: boolean;
  }) => invoke<JudgeResult>("run_samples", { req }),
  detectToolchains: () => invoke<Toolchain[]>("detect_toolchains"),

  // plans
  generatePlan: (req: {
    reason?: string;
    focusTags?: string[];
    count?: number;
    platforms?: string[];
  }) => invoke<Plan>("generate_plan", { req }),
  getActivePlan: () => invoke<Plan | null>("get_active_plan"),
  setPlanItemStatus: (itemId: string, status: string) =>
    invoke<void>("set_plan_item_status", { itemId, status }),

  // submissions / analysis
  submitSolution: (req: {
    problemId: string;
    code: string;
    language?: string;
    autoSubmit?: boolean;
  }) => invoke<SubmitResult>("submit_solution", { req }),
  captureVerdict: (req: {
    problemId: string;
    code: string;
    language?: string;
    sinceEpoch?: number;
  }) => invoke<SubmitResult>("capture_verdict", { req }),
  recordManualSubmission: (req: {
    problemId: string;
    code: string;
    verdict: string;
    language?: string;
    timeMs?: number;
    memoryKb?: number;
  }) => invoke<Submission>("record_manual_submission", { req }),
  getSubmission: (submissionId: string) =>
    invoke<Submission>("get_submission", { submissionId }),
  listSubmissions: (limit = 50) =>
    invoke<SubmissionRow[]>("list_submissions", { limit }),
  getAnalysis: (submissionId: string) =>
    invoke<Analysis | null>("get_analysis", { submissionId }),
  reanalyzeSubmission: (submissionId: string) =>
    invoke<Analysis>("reanalyze_submission", { submissionId }),

  // reports
  generateReport: (req: { kind: string; periodStart?: string; periodEnd?: string }) =>
    invoke<Report>("generate_report", { req }),
  listReports: () => invoke<ReportSummary[]>("list_reports"),
  getReport: (reportId: string) => invoke<Report>("get_report", { reportId }),
  deleteReport: (reportId: string) => invoke<void>("delete_report", { reportId }),
  exportReport: (req: {
    reportId: string;
    format: "md" | "html" | "pdf";
    content?: string;
    path?: string;
  }) => invoke<string>("export_report", { req }),
  getPrintReportId: () => invoke<string | null>("get_print_report_id"),
  printReportWindow: () => invoke<void>("print_report_window"),
  closePrintWindow: () => invoke<void>("close_print_window"),

  // chat
  listConversations: () => invoke<Conversation[]>("list_conversations"),
  createConversation: () => invoke<Conversation>("create_conversation"),
  deleteConversation: (id: string) => invoke<void>("delete_conversation", { id }),
  getMessages: (conversationId: string) =>
    invoke<ChatMessage[]>("get_messages", { conversationId }),
  sendMessage: (conversationId: string, text: string) =>
    invoke<SendMessageResult>("send_message", { conversationId, text }),
  cancelMessage: (messageId: string) => invoke<boolean>("cancel_message", { messageId }),
  confirmTool: (requestId: string, approved: boolean) =>
    invoke<boolean>("confirm_tool", { requestId, approved }),

  // demo
  seedDemoData: () => invoke<string>("seed_demo_data"),
};

export function errText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object") {
    const anyError = error as Record<string, unknown>;
    const message = typeof anyError.message === "string" ? anyError.message : "";
    const detail = typeof anyError.detail === "string" ? anyError.detail : "";
    if (message) return detail ? `${message}（${detail}）` : message;
    try {
      return JSON.stringify(error);
    } catch {
      return "未知错误";
    }
  }
  return String(error ?? "未知错误");
}

export const events = {
  onAgentDelta: (cb: (e: AgentDeltaEvent) => void): Promise<UnlistenFn> =>
    listen<AgentDeltaEvent>("agent://delta", (e) => cb(e.payload)),
  onAgentReasoning: (cb: (e: AgentDeltaEvent) => void): Promise<UnlistenFn> =>
    listen<AgentDeltaEvent>("agent://reasoning", (e) => cb(e.payload)),
  onAgentTool: (cb: (e: AgentToolEvent) => void): Promise<UnlistenFn> =>
    listen<AgentToolEvent>("agent://tool", (e) => cb(e.payload)),
  onAgentDone: (cb: (e: AgentDoneEvent) => void): Promise<UnlistenFn> =>
    listen<AgentDoneEvent>("agent://done", (e) => cb(e.payload)),
  onAgentConfirm: (cb: (e: ToolConfirmation) => void): Promise<UnlistenFn> =>
    listen<ToolConfirmation>("agent://confirm", (e) => cb(e.payload)),
  onAutomationState: (cb: (e: AutomationEvent) => void): Promise<UnlistenFn> =>
    listen<AutomationEvent>("automation://state", (e) => cb(e.payload)),
  onAutomationExternal: (cb: (e: { platform: string; url: string }) => void): Promise<UnlistenFn> =>
    listen<{ platform: string; url: string }>("automation://external", (e) => cb(e.payload)),
  onSyncProgress: (cb: (e: SyncProgress) => void): Promise<UnlistenFn> =>
    listen<SyncProgress>("sync://progress", (e) => cb(e.payload)),
  onAnalysisUpdated: (cb: (e: { submissionId: string }) => void): Promise<UnlistenFn> =>
    listen<{ submissionId: string }>("analysis://updated", (e) => cb(e.payload)),
  onPlanUpdated: (cb: (e: { plan?: Plan }) => void): Promise<UnlistenFn> =>
    listen<{ plan?: Plan }>("plan://updated", (e) => cb(e.payload)),
  onSubmissionUpdated: (cb: (e: { id: string }) => void): Promise<UnlistenFn> =>
    listen<{ id: string }>("submission://updated", (e) => cb(e.payload)),
};

export type { Sample };
