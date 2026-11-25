# ICPC 备赛工作站

[![CI](https://github.com/webstorm-dxy/icpc-workstation/actions/workflows/ci.yml/badge.svg)](https://github.com/webstorm-dxy/icpc-workstation/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

一个本地优先的 ICPC 训练工作台：**主页就是内置 ICPC Agent 的对话框**。它把 OJ 登录/同步、训练数据面板、Agent 推荐题目、C++ 编辑器、样例测试、浏览器自动化提交、提交后代码分析、训练计划刷新和个人报告导出串成一个闭环。

> 平台：macOS（完整验证）。代码保持跨平台，Windows/Linux 未做运行时 QA。
> 默认分析模型：OpenCode Go `deepseek-v4.1-flash`（DeepSeek 直连预设为 `deepseek-flash`）。

## 功能

- **首次引导**：选择 OJ（Codeforces / AtCoder / 洛谷）→ 应用内登录 → 配置 Agent 提供商 → 首次同步。
- **Agent 对话**：流式输出、工具调用卡片、写操作/自动化操作人工确认、可停止。
- **数据面板**：rating 趋势、通过数与通过率、标签能力（Elo）、近期提交、代码分析摘要、计划进度。
- **训练计划**：Agent 从本地题池挑选 6–10 道未做题，给出理由、目标标签、难度带与预计用时；可以按平台分配题数（如 Codeforces 4 道 + 洛谷 2 道），生成时严格按配额出题，题池不足会在方案里注明并用其他平台补齐；每次提交后自动刷新并沿用上次的配额。
- **题目工作区**：题面/样例、CodeMirror C++ 编辑器、本地编译与样例测试、草稿自动保存。
- **LSP 编辑器**：内置 clangd 语言服务器（LSP over Tauri IPC），提供语义补全、悬停文档、实时诊断、跳转定义与格式化；与本地片段/关键字补全合并显示。未安装 clangd 时自动退回纯本地补全。
- **浏览器自动化提交**：Tauri 内嵌子 WebView；支持自动填表与可选自动点击（洛谷走站内提交接口，不受异步编辑器影响）；遇到验证码/风控自动暂停并交给用户；避免重复提交。
- **提交后分析**：无论 AC 与否都生成结构化分析（根因、证据、修复步骤、复杂度、学习点），并更新标签能力与训练计划。
- **报告**：Agent 生成训练报告，支持 Markdown / 自包含 HTML / PDF（系统打印对话框）。
- **本地优先**：SQLite + 系统钥匙串；无自建服务器、无遥测。

## 技术栈

Tauri 2.12（多 WebView，`unstable`）· Rust 1.95 · React 19.3 · antd 6.7 · CodeMirror 6 · ECharts 6 · SQLite/sqlx · KaTeX。

## 开发

需要 Node 22+、pnpm、Rust、Xcode Command Line Tools；macOS 判题推荐安装 Homebrew `gcc`（提供 GNU `g++-16`）。

```bash
source scripts/env.sh   # 项目内缓存（.cargo-home/.pnpm-store/.tmp），规避沙箱权限
pnpm install
pnpm tauri dev
```

常用命令：

```bash
pnpm typecheck
pnpm test
pnpm build
cd src-tauri && cargo test
cd src-tauri && cargo clippy --all-targets -- -D warnings
./scripts/check.sh
```

### 演示数据

首次启动前设置：

```bash
ICPC_AUTOSEED=1 ICPC_START_ROUTE=/dashboard pnpm tauri dev
```

会写入虚构账号/题目/提交/计划/报告并完成引导。正式使用时不要带此环境变量；可在设置 → 数据中清除。

## 首次使用

1. 启动后按引导选择平台。
2. 分别打开登录页并登录；点“验证登录”。
3. 配置 OpenCode Go（默认）或 DeepSeek / OpenAI / Anthropic / 自定义兼容端点。API Key 写入系统钥匙串。
4. 首次同步；洛谷可能需要人机验证。
5. 进入工作站，从 Agent 对话或训练页开始。

### OpenCode Go 注意事项

- Base URL：`https://opencode.ai/inference/go`
- 聊天：`/openai/v1/chat/completions`；模型列表：`/v1/models`；用量：`/v1/usage`
- 请求会携带稳定的 `x-opencode-session` 与应用 UA，用于路由和 prompt cache。
- DeepSeek V4.1 Flash 需要在 OpenCode Console → Privacy → Regions 选择 **Global**，否则可能 403。
- 401/403 不重试；429 显示 Go 用量并退避；设置页可查看 rolling/weekly/monthly 用量。

## OJ 支持与反爬

| 平台 | 读取 | 提交 | 说明 |
|---|---|---|---|
| Codeforces | 官方 API | 内置浏览器填表/提交 | 限速 1 请求/2s；题池缓存 24h |
| AtCoder | kenkoooo 公开 API + rating 历史 JSON | 内置浏览器 | 提交冷却 5 分钟 |
| 洛谷 | 页面内 same-origin fetch（DOM 兜底） | 内置浏览器 | WAF/验证码频繁；可人工接管与手动录入 |

**反爬不是被绕过的对象**：验证码、登录失效、Cloudflare 挑战会暂停自动化，把页面显示给用户处理，用户确认后继续。不会自动重试可能造成重复提交的点击。平台页面结构变化时，可在应用内手动录入提交结果。

## 本地判题

- 默认 `gnu++17`、`-O2`，支持切换 GNU++20/23。
- 自动探测 `g++-16` / `g++` / `clang++`；Apple clang 缺少 `bits/stdc++.h` 时使用内置兼容头。
- 编辑器缩进固定 4 空格（Tab、回车自动缩进、整块重排一致），模板与补全片段同宽；LSP 的 clang-format 也用 `IndentWidth: 4`。
- 语言服务器自动探测 PATH 与 Homebrew/Xcode 常见路径下的 `clangd`；也可用环境变量 `ICPC_LSP_SERVER` 指定。编辑器工具栏的标签可切换服务器、重启或关闭。
- 编译 20s 超时；运行按题目时限 + 1.5s 并 kill；输出上限 1MB；行尾空白可忽略。
- 代码以当前用户权限执行。可在设置中调整编译器；暂未实现强沙箱。

## 隐私与安全

- API Key 仅存 macOS Keychain；数据库/日志无明文。
- OJ WebView 不获得应用命令权限；自动化通过原生 `eval_with_callback` 读取页面，不向远程页面注入 Tauri IPC。
- 代码是否发给模型由“允许把代码发给模型做分析”开关控制。
- 所有训练数据、报告、草稿都在本机应用数据目录。

## 已知限制

- 洛谷的数据结构与接口可能变化，必要时手动接管。
- PDF 通过系统打印对话框“存储为 PDF”，不是静默生成。
- 未实现 Windows/Linux 运行时验证、移动端、云端同步、自动更新。
- 提交自动化依赖 OJ 页面结构；失败时走人工兜底。

## 构建产物

```bash
pnpm tauri build --debug --bundles app
open "src-tauri/target/debug/bundle/macos/ICPC Workstation.app"
```

Debug bundle 约 68 MB，未签名/未公证；正式分发前需要 Developer ID 签名与 notarization。

## 目录

```
src/                 React UI（引导/对话/数据/训练/工作区/报告/设置）
src-tauri/src/       Rust 后端（LLM/Agent/平台/自动化/判题/分析/报告）
src-tauri/migrations SQLite schema
src-tauri/prompts/   Agent 提示词
docs/                架构、OJ 适配器、Agent、计划与进度
```

## 参与

欢迎 Issue 与 Pull Request。提交前请先跑通 `./scripts/check.sh`（前端 typecheck/test + Rust fmt/clippy/test），CI 会执行同样的检查。

请注意：

- 不要提交 API Key、OJ Cookie、`.appdata/`、`.cargo-home/`、`.pnpm-store/`。
- 密钥只进系统钥匙串，日志中不得出现 Authorization / Cookie 值。
- 涉及真实 OJ 提交的自动化需要用户显式确认，不要加入会自动向 OJ 提交的测试。

## 许可证

[MIT](LICENSE) © 2026 webstorm-dxy
