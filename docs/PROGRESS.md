# 进度与验证

## 已完成

- Tauri 2 单窗口多 webview：`ui` + 按需 `oj-*` + `print`；主窗口无初始 webview。
- SQLite 迁移、Keychain、结构化错误、日志、IPC 命令与事件。
- 五类 provider（OpenCode Go / DeepSeek / OpenAI / Anthropic / 自定义）与四种角色；OpenCode Go 端点、`x-opencode-session`、UA、usage、401/403 提示。
- Agent 流式对话、工具循环、确认（oneshot）、停止、对话持久化。
- Codeforces / AtCoder 同步与题面解析；洛谷页面内 same-origin fetch 同步与题面兜底。
- 数据面板（ECharts）、训练计划生成/校验/确定性兜底、计划项状态。
- 题目工作区、C++ 补全、本地判题、草稿、手动录入。
- 提交自动化：CF/AtCoder/洛谷填表、点击、判定轮询、验证码人工接管、冷却与重复代码保护。
- 分析流水线、标签 Elo、提交后计划刷新、启动恢复待分析任务。
- 报告生成、Markdown/HTML/PDF 导出、打印窗口。
- 首次引导、设置、演示数据。

## 已执行的验证

- `pnpm typecheck`、`pnpm lint`、`pnpm build` 通过。
- `pnpm test`：3 个前端测试通过。
- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`：15 个 Rust 测试通过；含真实 g++-16 的样例编译/运行测试。
- 启动应用并逐页读取 webview DOM：chat / dashboard / training / problem / reports / settings / onboarding 均渲染出预期内容。
- 通过 `ICPC_TEST_OPEN_URL` 实测 OJ 子 webview：Codeforces 首页 `readyState=complete`、标题 `Codeforces`、正文 18k+ 字符，说明 `add_child` 与 `eval_with_callback` 链路可用。
- 演示数据：21 题、12 提交、1 计划、1 报告；引导完成状态写入。
- 生产 bundle：`pnpm tauri build --debug --bundles app` 成功（67.89 MiB）；直接运行 bundle 内二进制并从 `tauri://localhost#/onboarding` 读取 DOM，引导页正常、无 JS 错误。
- macOS bundle identifier 已改为 `com.icpcworkstation.desktop`（避免 `.app` 后缀冲突）。

## 环境注意

- 在 DSH 的 workspace-write 沙箱内启动应用时，macOS WebKit 无法写入 `~/Library/WebKit`，会出现 “could not create directory … Operation not permitted”。页面仍可渲染，但 **Cookie / localStorage 不持久**。真实使用请从 Finder/普通终端启动构建后的 `.app`（或在允许的沙箱外运行），登录态才能保留。
- 开发用 `ICPC_DATA_DIR=.appdata` 把数据库放在项目内；构建后的正式应用使用系统应用数据目录。
- 判题未实现强沙箱；只做了超时、输出上限、进程组 kill。
- 真实 OJ 提交与真实模型调用需要用户账号/Key，未做自动化 CI。

## AOCI

- 已 `aoci init`（zh-CN）、`aoci scan` 建立基线；当前基线 126 个受管文件。
- 已将 `.appdata*/.cargo-home/.pnpm-store/.tmp/target` 等开发产物排除；通过一次 `managed_scope_change_apply` 移除 `.appdata-onboarding` 调试文件。
- 当前 `aoci status`：`authoring_required`，missing 68；需要项目专用 `aoci-icpc` MCP 会话执行 `aoci_maintain` → 读源码 → 创作条目 → `aoci_update_entry`。
- 已在 DSH bundle `cordis.patch.yml` 追加 `aoci-icpc`（保留原虎符 `aoci` 绑定；备份 `.bak`）。**需要重启 DSH 会话**后生效；重启后先读 `aoci_rules` 与完整 `aoci_overview`，再继续条目创作。
- 当前索引状态：missing 68、stale 若干（初始化后又改了 identifier/docs/tests）；这是未完成 authoring 的正常状态。
- `aoci check` 目前会因 authoring_required 非零退出，这是预期的未完成索引状态，不是代码失败。

## 已知限制

- 洛谷接口/页面结构可能变化；已提供人工接管与手动录入。
- 前端单 chunk 约 3.6MB；后续可按路由拆分。
- 未实现 Windows/Linux 运行时验证、移动端、云同步、自动更新。
- 报告 PDF 依赖系统打印对话框。
