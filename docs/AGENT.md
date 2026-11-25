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

- 对话 system = chat.md + 训练快照（账号、薄弱标签、active 计划、最近 6 次提交）。
- 训练计划使用 `recommend` 角色：最近 30 条提交、最多 8 份可用源码、已有分析与候选题目；模型先输出 `diagnosis`，再输出 `rationale` 和题目。原始输出通过 `plan://agent` 流式展示；流式中断会用同一 Agent 重试，失败则报错，不做无 Agent 回退。
- 来源配额：请求里的 `platforms` 是 `[{platform, count}]`。后端会先按平台限定候选池（每道配额给 5 个候选）、把配额写进 prompt，再在 `normalize_payload` 里强制执行：模型多给的平台题会被丢弃，少给/没给的从同平台候选按分数补齐；某平台题池不足时在方案里写明“实际安排 x/y 道，差额由其他平台补齐”，并保证总数不变。候选按平台分别查询（每平台 250 条），否则上万道 Codeforces 会在单条全局查询里占满名额、让其他平台在进入配额逻辑前就被挤掉。配额会存进 settings 的 `plan.quotas`，提交后的自动刷新沿用该配额；传空数组即清除配额。
- 最近 24 条 user/assistant 消息。
- 工具结果截断到 12k 字符。
- 推理内容只实时展示，不写库、不回传。

## 事件

`agent://delta`、`agent://reasoning`、`agent://tool`、`agent://confirm`、`agent://done`。
