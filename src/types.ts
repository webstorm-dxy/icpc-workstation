export interface OnboardingState {
  complete: boolean;
  step: number;
  providerReady: boolean;
  localOnly: boolean;
  platforms: string[];
  configuredPlatforms: string[];
}

export interface Provider {
  id: string;
  kind: string;
  name: string;
  baseUrl: string;
  hasKey: boolean;
  enabled: boolean;
  config: Record<string, unknown>;
}

export interface ModelInfo {
  id: string;
  name: string;
  context?: number | null;
}

export interface ProviderTestResult {
  ok: boolean;
  models: ModelInfo[];
  message: string;
}

export interface RoleModel {
  role: string;
  providerId?: string | null;
  modelId?: string | null;
  providerName?: string | null;
  providerKind?: string | null;
}

export interface Platform {
  id: string;
  name: string;
  description: string;
  loginUrl: string;
  needsHandle: boolean;
  handleLabel: string;
  loggedIn: boolean;
  handle?: string | null;
  capabilities: string[];
}

export interface Settings {
  theme: string;
  defaultLanguage: string;
  compilerPath?: string | null;
  compilerStandard: string;
  autoSubmit: boolean;
  requireSubmitConfirm: boolean;
  allowCodeUpload: boolean;
  luoguSyncDepth: number;
  autoRefreshPlan: boolean;
  reportDir?: string | null;
  mockMode: boolean;
}

export interface AccountCard {
  platform: string;
  name: string;
  handle?: string | null;
  rating: number;
  maxRating: number;
  solvedCount: number;
  lastSyncedAt?: string | null;
  loggedIn: boolean;
  meta: Record<string, unknown>;
}

export interface RatingPoint {
  at: string;
  rating: number;
  platform: string;
}

export interface TagStat {
  tag: string;
  rating: number;
  attempts: number;
  solved: number;
}

export interface SubmissionRow {
  id: string;
  platform: string;
  problemId: string;
  problemTitle: string;
  verdict: string;
  language?: string | null;
  submittedAt: string;
  analysisStatus: string;
  timeMs?: number | null;
}

export interface AnalysisFeed {
  submissionId: string;
  problemId: string;
  problemTitle: string;
  verdict: string;
  summary: string;
  category: string;
  createdAt: string;
}

export interface PlanItem {
  id: string;
  problemId: string;
  title: string;
  platform: string;
  url: string;
  rating?: number | null;
  tags: string[];
  kind: string;
  reason: string;
  expectedMinutes?: number | null;
  status: string;
}

export interface Plan {
  id: string;
  status: string;
  rationale: string;
  createdAt: string;
  items: PlanItem[];
}

export interface Totals {
  solved: number;
  submissions: number;
  accepted: number;
  acceptanceRate: number;
  streakDays: number;
}

export interface Dashboard {
  accounts: AccountCard[];
  ratingPoints: RatingPoint[];
  tagStats: TagStat[];
  recentSubmissions: SubmissionRow[];
  analyses: AnalysisFeed[];
  activePlan: Plan | null;
  totals: Totals;
}

export interface Sample {
  input: string;
  output: string;
}

export interface Problem {
  id: string;
  platform: string;
  nativeId: string;
  title: string;
  url: string;
  rating?: number | null;
  normalizedRating?: number | null;
  tags: string[];
  skillTags: string[];
  timeLimitMs?: number | null;
  memoryLimitMb?: number | null;
  statementMd: string;
  samples: Sample[];
  solved: boolean;
  attempted: boolean;
}

export interface Candidate {
  id: string;
  platform: string;
  title: string;
  rating?: number | null;
  tags: string[];
  attempted: boolean;
}

export interface Toolchain {
  path: string;
  kind: string;
  version: string;
  supportsBits: boolean;
}

export interface SampleResult {
  index: number;
  status: string;
  timeMs: number;
  stdout: string;
  stderr: string;
  expected: string;
  diff: string;
}

export interface JudgeResult {
  ok: boolean;
  compileOk: boolean;
  compileOutput: string;
  samples: SampleResult[];
  totalMs: number;
  compiler?: string | null;
}

export interface Analysis {
  submissionId: string;
  summary: string;
  failureCategory: string;
  rootCause: string;
  bugPatterns: string[];
  evidence: string[];
  complexity: Record<string, unknown>;
  fixSteps: string[];
  nextPracticeTags: string[];
  learningPoints: string[];
  confidence: number;
  createdAt: string;
  providerId?: string | null;
  modelId?: string | null;
}

export interface Submission {
  id: string;
  platform: string;
  problemId: string;
  problemTitle: string;
  verdict: string;
  language?: string | null;
  timeMs?: number | null;
  memoryKb?: number | null;
  submittedAt: string;
  analysisStatus: string;
  needsHuman: boolean;
  message?: string | null;
  analysis?: Analysis | null;
}

export interface SubmitResult {
  needsHuman: boolean;
  message: string;
  submission?: Submission | null;
  automationStatus: string;
}

export interface AutomationBounds {
  x: number;
  y: number;
  width: number;
  height: number;
  /** UI webview CSS viewport height, used to correct the native top inset. */
  viewportHeight?: number;
}

export interface AutomationState {
  platform: string;
  visible: boolean;
  url?: string | null;
  status: string;
  message: string;
  needsHuman: boolean;
  hint?: string | null;
}

export interface Conversation {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
}

export interface ToolCall {
  id: string;
  name: string;
  status: string;
  summary: string;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  createdAt: string;
  toolCalls: ToolCall[];
  usage?: Record<string, unknown> | null;
}

export interface SendMessageResult {
  conversationId: string;
  userMessageId: string;
  assistantMessageId: string;
}

export interface ToolConfirmation {
  requestId: string;
  tool: string;
  summary: string;
  args: Record<string, unknown>;
}

export interface ReportSummary {
  id: string;
  kind: string;
  title: string;
  periodStart?: string | null;
  periodEnd?: string | null;
  createdAt: string;
}

export interface Report {
  id: string;
  kind: string;
  title: string;
  periodStart?: string | null;
  periodEnd?: string | null;
  contentMd: string;
  data: Record<string, unknown>;
  createdAt: string;
}

export interface SyncProgress {
  platform: string;
  stage: string;
  message: string;
  percent: number;
  status: string;
}

export interface PlatformLoginResult {
  loggedIn: boolean;
  handle?: string | null;
  message: string;
}

export interface Draft {
  language: string;
  code: string;
}

export interface ProviderUsageMeter {
  window: string;
  percent: number;
  status: string;
  resetsAt?: string | null;
}

export interface ProviderUsage {
  meters: ProviderUsageMeter[];
  message?: string | null;
}

export type AgentDeltaEvent = {
  conversationId: string;
  messageId: string;
  delta: string;
};

export type AgentToolEvent = {
  conversationId: string;
  messageId: string;
  tool: ToolCall;
};

export type AgentDoneEvent = {
  conversationId: string;
  messageId: string;
  content: string;
  usage?: Record<string, unknown> | null;
  error?: string | null;
};

export type AutomationEvent = {
  platform: string;
  status: string;
  message: string;
  needsHuman: boolean;
  hint?: string | null;
  url?: string | null;
};

export interface AppInfo {
  version: string;
  dataDir: string;
  model: string;
}
