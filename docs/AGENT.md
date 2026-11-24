# Agent

## 角色

| role | 用途 | 默认模型 |
|---|---|---|
| `chat` | 主页对话 | OpenCode Go `deepseek-v4.1-flash` |
| `recommend` | 生成训练计划 | 同上 |
| `analyze` | 提交后代码分析 | 同上 |
| `report` | 个人报告 | 同上 |

DeepSeek 直连默认 `deepseek-flash`。设置 → Agent 角色可逐角色覆盖，也可配置其他 provider。

## 工具

只读：`get_training_overview`、`get_skill_profile`、`list_candidates`、`get_problem`、`get_recent_submissions`。
有副作用（需确认）：`generate_training_plan`、`refresh_training_plan`、`sync_platform`、`run_samples`、`submit_draft`、`generate_report`。

`submit_draft` 只填表，不自动点击；真正的提交由用户在自动化面板确认。

## 提示词

- `src-tauri/prompts/chat.md`：对话系统提示（工具规范、防注入、表达风格）。
- `analysis.rs`：分析师 system + JSON schema。
- `plan.rs`：规划师 system + JSON schema。
- `reports.rs`：报告撰写 system。

提示词版本记录在 `analyses.prompt_version`，当前 `analyst/v1`。

## 上下文

- system = chat.md + 训练快照（账号、薄弱标签、active 计划、最近 6 次提交）。
- 最近 24 条 user/assistant 消息。
- 工具结果截断到 12k 字符。
- 推理内容只实时展示，不写库、不回传。

## 事件

`agent://delta`、`agent://reasoning`、`agent://tool`、`agent://confirm`、`agent://done`。
