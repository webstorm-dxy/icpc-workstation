# 架构

## 进程与窗口模型

```
Tauri App (Rust)
└─ main Window (no webview at build time)
   ├─ child webview "ui"   → React UI (full window, auto-resize)
   └─ child webview "oj-*" → OJ pages, created on demand, positioned over the automation panel
```

- 主窗口由 `WindowBuilder` 创建，**不带** webview；`setup` 中通过 `Window::add_child` 添加 `ui`。
- OJ 子 webview 使用 `add_child` + `set_position/set_size`，由前端把面板的 `getBoundingClientRect()` 上报给 Rust。
- 自动化读取页面使用 `Webview::eval_with_callback`：Rust 执行同步 JS，JS 返回 JSON 字符串，Rust 解码。**不向 OJ 页面注入可用的 Tauri IPC**。
- 导航白名单按平台配置；未知链接通过 `automation://external` 事件交给系统浏览器。
- `print` 是独立 `WebviewWindow`，只用于报告打印/另存 PDF。

为什么不用 Playwright：包体、登录态、验证码人工接管和“用户能看到真实页面”是本项目的核心交互；内嵌 WebView 能同时满足。代价是每个平台的 DOM 脚本需要维护。

## Rust 模块

| 模块 | 职责 |
|---|---|
| `state.rs` | 共享状态、日志、数据目录、待分析任务恢复 |
| `db.rs` | SQLite 连接池、迁移、settings 读写、重置 |
| `llm.rs` | 统一消息/工具协议、OpenAI Chat Completions、Anthropic Messages、SSE 解析、错误映射、usage |
| `provider.rs` | provider/role 解析、默认端点与模型、Keychain 读取 |
| `agent.rs` | 对话循环、流式事件、工具执行、确认、取消 |
| `platforms.rs` | 三平台同步、题面抓取、登录探测、题池/样例解析 |
| `automation.rs` | 子 webview 生命周期、eval/等待/轮询、状态事件 |
| `submission.rs` | 填表/点击/判定轮询/人工兜底/手动录入 |
| `judge.rs` | 工具链探测、编译、样例执行、超时/输出限制、diff |
| `lsp.rs` | clangd 探测与进程生命周期、LSP over stdio 分帧、JSON-RPC 中继、工作区与编译旗标 |
| `analysis.rs` | 提交后结构化分析、确定性兜底、标签 Elo、恢复待分析任务 |
| `plan.rs` | 候选池、LLM 推荐、约束校验、确定性兜底、计划刷新 |
| `reports.rs` | 数据聚合、报告生成、模板兜底、导出数据 |
| `commands.rs` | Tauri 命令、事件、演示数据 |

## 代码编辑器与 LSP

题目页的 C++ 编辑器是 CodeMirror 6，语言能力由 clangd 提供：

```
webview (src/lib/lsp.ts)                     Rust (src-tauri/src/lsp.rs)
LanguageServerClient ── invoke send_lsp_message ──▶ 写 clangd stdin
        ▲                                          clangd (per problem+dialect)
        └──── event lsp://message ◀────────────── 读 clangd stdout
```

- 传输不走 WebSocket：CSP 只允许 `'self'`，且不额外开本地端口；JSON-RPC 帧由 Tauri IPC 搬运。`send` 在前端串行排队，否则 `didOpen` 可能晚于 `didChange` 到达。
- 每个「题目 + `-std` + 服务器路径」对应一个 clangd 进程，由前端缓存复用（React StrictMode 双挂载、页面来回切换不会反复起进程）；超过 2 个时按最近使用淘汰，应用退出时 `RunEvent::Exit` 统一回收。
- Rust 只做三件事：探测服务器、准备 `<数据目录>/lsp/<problem>/`（`main.cpp`、`compile_flags.txt`、`bits/stdc++.h` 兼容层）、分帧转发。协议状态机全在 `src/lib/lsp.ts`。
- `compile_flags.txt` 固定为 `-x c++ / -std=<方言> / -I<工作区>`：`-I` 让 `#include <bits/stdc++.h>` 命中兼容层，与本机判题的行为一致（macOS 的 clang 没有 libstdc++ 的 bits 目录）。
- 补全来源合并：本地精选片段/关键字（`cppCompletion`）与 clangd 语义补全在同一次查询里合并，按 label 去重，`filter: false` 交给服务端排序；两边的替换区间可能不同，本地项因此被固定在自己的区间上。
- 依赖包自带的 `languageServerWithTransport()` 未被使用：它会自带 `autocompletion({ override })`，与本地的 override 冲突（facet 合并会抛错）。
- clangd 不传 `--stdio`（stdio 是默认传输，clangd 没有该开关），并关闭 background index。
- 语言服务器不可用时编辑器自动退回纯本地补全；工具栏标签可切换服务器、重启或关闭。

## 数据

SQLite（WAL）位于应用数据目录 `workstation.db`。开发时 `ICPC_DATA_DIR` 可指向项目内 `.appdata/`。

核心表：`providers`、`role_models`、`platform_accounts`、`problems`、`submissions`、`submission_code`、`analyses`、`skill_profiles`、`training_plans`/`plan_items`、`conversations`/`messages`、`reports`、`run_results`、`drafts`、`platform_cache`。

- ID 使用 UUIDv7；时间统一 ISO-8601 UTC。
- 题目规范 ID：`cf:1791F`、`atcoder:abc300_f`、`luogu:P3373`。
- 提交去重：`(platform, native_id)` 唯一；代码 SHA-256 用于 10 分钟重复提交保护。
- 评分：CF 直接 rating；AtCoder 使用 kenkoooo difficulty；洛谷 1–7 映射到 800–2800。

## LLM 提供商

统一 `ChatRequest`/`LlmMessage`/`ToolSpec`。两个方言：

- **OpenAI Chat Completions**：OpenCode Go、DeepSeek、OpenAI、自定义。
- **Anthropic Messages**：Anthropic（system 独立、content blocks、tool_use/tool_result）。

### OpenCode Go 特殊处理

| 项 | 值 |
|---|---|
| Base URL | `https://opencode.ai/inference/go` |
| Chat | `POST /openai/v1/chat/completions` |
| Models | `GET /v1/models` |
| Usage | `GET /v1/usage` |
| 认证 | `Authorization: Bearer` |
| 会话 | `x-opencode-session: ses_<conversation>`（稳定） |
| UA | `ICPCWorkstation/<version>` |

- 401 → Key 无效；403 → 订阅/模型/地区限制（提示 Privacy → Regions → Global）；429 → 显示 rolling/weekly/monthly 并退避。
- `stream_options` 只在 OpenAI/OpenCode Go 上发送；DeepSeek 不发送。
- 结构化输出：OpenAI 兼容用 `response_format: json_object`；Anthropic 用强制 `submit_result` tool；失败一次修复重试。

## Agent 循环

1. 读取最近 24 条消息 + 训练快照（账号、薄弱标签、active 计划、最近提交）拼 system prompt。
2. 流式请求；文本增量经 `agent://delta` 推送，推理增量经 `agent://reasoning`。
3. 若返回 tool_calls：先推送 `agent://tool` running，执行工具（必要时弹确认），再推送结果，把 tool 结果加入上下文，进入下一轮（最多 6 步）。
4. 结束时把最终文本与工具摘要写入 `messages`，推送 `agent://done`。
5. 停止：`cancel_message` 设置 AtomicBool，流循环与下一步检查。

确认机制：需要确认的工具在 Rust 里创建 oneshot，推送 `agent://confirm`，前端 Modal 调用 `confirm_tool`；120 秒未响应视为拒绝。

## 提交状态机

```
Opening → VerifyingSession → Filling → (PreSubmitConfirm) → Clicking
→ AwaitingVerdict → Completed | NeedsHuman | Failed | Cancelled
```

- 平台/提交串行；冷却（CF/洛谷 60s，AtCoder 300s）；相同代码 10 分钟内拒绝。
- 洛谷自动提交不走 DOM：在已登录页面里 `POST /fe/api/problem/submit/{pid}`（带 CSRF 和 `{code, lang, enableO2}`），拿到 `rid` 后轮询 `/record/{rid}`；填表流程只用于“等用户确认再提交”的场景。
- 检测验证码/登录页/403：设置 `NeedsHuman`，显示页面，等待用户“验证登录/继续”后由用户重试或调用 `capture_verdict`。
- 点击后不盲目重试；若拿不到判定，保留“读取结果”和“手动录入”。

## 本地判题

`g++-16` → `g++` → `clang++` 探测；编译 20s 超时；运行超时 = 题目时限 + 1.5s；输出上限 1MB；逐样例归一化比较。Apple clang 缺 `bits/stdc++.h` 时写兼容头并 `-I`。

## 分析 / 计划 / 报告

- 分析：编辑证据 + LLM JSON → 落库；失败用确定性模板；更新标签 Elo；AC 时标记计划项完成；触发计划刷新（20s 防抖）。
- 计划：画像 + 候选 60 题 → LLM 选 6–10 题 → 校验（存在、未做、去重、数量）→ 失败用确定性贪心。
- 报告：聚合数据 → reporter 模型 Markdown；失败用模板。HTML 由前端 unified 生成自包含文件；PDF 走打印窗口。
