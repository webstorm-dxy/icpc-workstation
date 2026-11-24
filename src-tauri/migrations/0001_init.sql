PRAGMA journal_mode=WAL;

CREATE TABLE IF NOT EXISTS app_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS providers (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  base_url TEXT NOT NULL,
  secret_ref TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  config_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS role_models (
  role TEXT PRIMARY KEY,
  provider_id TEXT NOT NULL,
  model_id TEXT NOT NULL,
  params_json TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS platform_accounts (
  platform TEXT PRIMARY KEY,
  handle TEXT,
  uid TEXT,
  display_name TEXT,
  avatar_url TEXT,
  rating INTEGER,
  max_rating INTEGER,
  solved_count INTEGER,
  logged_in INTEGER NOT NULL DEFAULT 0,
  meta_json TEXT NOT NULL DEFAULT '{}',
  last_synced_at TEXT
);

CREATE TABLE IF NOT EXISTS sync_runs (
  id TEXT PRIMARY KEY,
  platform TEXT NOT NULL,
  kind TEXT NOT NULL,
  status TEXT NOT NULL,
  started_at TEXT NOT NULL,
  finished_at TEXT,
  stats_json TEXT NOT NULL DEFAULT '{}',
  error_code TEXT,
  error_msg TEXT
);

CREATE TABLE IF NOT EXISTS problems (
  id TEXT PRIMARY KEY,
  platform TEXT NOT NULL,
  native_id TEXT NOT NULL,
  title TEXT NOT NULL,
  url TEXT NOT NULL,
  rating INTEGER,
  normalized_rating INTEGER,
  tags_json TEXT NOT NULL DEFAULT '[]',
  skill_tags_json TEXT NOT NULL DEFAULT '[]',
  time_limit_ms INTEGER,
  memory_limit_mb INTEGER,
  statement_md TEXT,
  samples_json TEXT NOT NULL DEFAULT '[]',
  content_hash TEXT,
  fetched_at TEXT,
  UNIQUE(platform, native_id)
);
CREATE INDEX IF NOT EXISTS idx_problems_platform_rating ON problems(platform, normalized_rating);

CREATE TABLE IF NOT EXISTS submissions (
  id TEXT PRIMARY KEY,
  platform TEXT NOT NULL,
  native_id TEXT,
  problem_id TEXT NOT NULL,
  verdict TEXT NOT NULL,
  language TEXT,
  code_sha256 TEXT,
  time_ms INTEGER,
  memory_kb INTEGER,
  submitted_at TEXT NOT NULL,
  captured_via TEXT NOT NULL DEFAULT 'sync',
  raw_json TEXT,
  analysis_status TEXT NOT NULL DEFAULT 'pending',
  UNIQUE(platform, native_id)
);
CREATE INDEX IF NOT EXISTS idx_submissions_problem ON submissions(problem_id, submitted_at DESC);
CREATE INDEX IF NOT EXISTS idx_submissions_time ON submissions(submitted_at DESC);

CREATE TABLE IF NOT EXISTS submission_code (
  submission_id TEXT PRIMARY KEY,
  code_text TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS analyses (
  id TEXT PRIMARY KEY,
  submission_id TEXT NOT NULL UNIQUE,
  provider_id TEXT,
  model_id TEXT,
  prompt_version TEXT,
  payload_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS skill_profiles (
  tag TEXT PRIMARY KEY,
  rating REAL NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  solved INTEGER NOT NULL DEFAULT 0,
  last_delta REAL NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS training_plans (
  id TEXT PRIMARY KEY,
  status TEXT NOT NULL,
  reason TEXT,
  source_submission_id TEXT,
  rationale_md TEXT,
  provider_id TEXT,
  model_id TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS plan_items (
  id TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL,
  problem_id TEXT NOT NULL,
  kind TEXT,
  reason TEXT,
  target_tags_json TEXT NOT NULL DEFAULT '[]',
  expected_minutes INTEGER,
  status TEXT NOT NULL DEFAULT 'pending',
  completed_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_plan_items_plan ON plan_items(plan_id);

CREATE TABLE IF NOT EXISTS conversations (
  id TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  role TEXT NOT NULL,
  content_md TEXT NOT NULL,
  tool_calls_json TEXT NOT NULL DEFAULT '[]',
  usage_json TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_id, created_at);

CREATE TABLE IF NOT EXISTS reports (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  title TEXT NOT NULL,
  period_start TEXT,
  period_end TEXT,
  content_md TEXT NOT NULL,
  data_json TEXT NOT NULL DEFAULT '{}',
  provider_id TEXT,
  model_id TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS run_results (
  id TEXT PRIMARY KEY,
  problem_id TEXT NOT NULL,
  submission_id TEXT,
  sample_index INTEGER NOT NULL,
  status TEXT NOT NULL,
  time_ms INTEGER,
  memory_kb INTEGER,
  stdout TEXT,
  stderr TEXT,
  expected TEXT,
  code_sha256 TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS drafts (
  problem_id TEXT PRIMARY KEY,
  language TEXT NOT NULL DEFAULT 'cpp',
  code_text TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_log (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS platform_cache (
  key TEXT PRIMARY KEY,
  body TEXT NOT NULL,
  fetched_at TEXT NOT NULL
);
