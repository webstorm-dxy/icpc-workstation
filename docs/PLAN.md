# 实施计划（摘要）

完整方案见会话归档。核心决策：

- macOS 优先的 Tauri 2 桌面应用；React 19 + antd 6；Rust 1.95；SQLite/sqlx。
- 内置 Agent 主页；OpenCode Go 默认 + DeepSeek，OpenAI/Anthropic 可选。
- Codeforces / AtCoder / 洛谷三平台；内嵌子 WebView 自动化，无 Playwright。
- CodeMirror 6 C++ 编辑 + 自定义补全；本地 GNU g++ 判题。
- 提交后必分析；分析驱动标签 Elo 与训练计划刷新；报告可导出 MD/HTML/PDF。
- AOCI 以项目专用 MCP 绑定维护（不在应用运行时使用）。

## 阶段

| 阶段 | 内容 | 状态 |
|---|---|---|
| P0 | 脚手架、Tauri 多 WebView、DB、Keychain、日志 | 完成 |
| P1 | 引导、provider、Agent 流式对话、确认 | 完成 |
| P2 | 三平台同步、数据面板 | 完成（真实账号待验证） |
| P3 | 训练计划、题目工作区、C++ 补全、本地判题 | 完成 |
| P4 | 提交自动化、人工接管、判定轮询 | 完成（真实 OJ 待验证） |
| P5 | 提交分析、技能画像、计划刷新 | 完成 |
| P6 | 报告生成与 MD/HTML/PDF 导出 | 完成 |
| P7 | 打磨、构建、文档、AOCI | 进行中 |

## 验收口径

见 README 与 `docs/PROGRESS.md`；真实 OJ 提交/实时 API 测试需要用户账号与明确同意，不进入自动化 CI。
