# OJ 适配器

## 能力矩阵

| 平台 | 登录 | 读取 | 题面/样例 | 提交 | 反爬 |
|---|---|---|---|---|---|
| Codeforces | `/enter` | 官方 API | HTML（`.problem-statement`、`.sample-test`） | 内置浏览器 + API 轮询 | Cloudflare 偶发 |
| AtCoder | `/login` | kenkoooo API + `/users/{handle}/history/json` | HTML（`#task-statement`、`pre`） | 内置浏览器 + kenkoooo 轮询 | 提交冷却 |
| 洛谷 | `/auth/login` | 页面内 same-origin `fetch`（`_contentOnly=1`） | `_contentOnly` JSON；DOM 兜底 | 站内提交接口 `POST /fe/api/problem/submit/{pid}` | WAF/验证码频繁 |

## 同步流程

1. 检查账号 handle/UID；没有则报 `E_PLATFORM_HANDLE`。
2. 刷新题池（CF/AtCoder 24h 缓存；洛谷按需 3 页）。
3. 读取账号资料/rating 历史。
4. 增量读取提交；`(platform, native_id)` 幂等 upsert。
5. 统计 distinct AC 数，写 `platform_accounts.last_synced_at`。
6. 全程 `sync://progress` 事件；同平台互斥。

## 自动化 API（JS 片段）

- `login_probe`：返回 `{logged, handle, uid}`。
- `challenge_probe`：检测验证码 iframe/Cloudflare/登录表单/“访问频繁”。
- 填表脚本：设置 problem/task select、language select、编辑器（CodeMirror 5 / Ace / Monaco / textarea / contenteditable），返回 `{ok, language, editor}`。
- 点击脚本：按文字/类型找提交按钮。
- POST 脚本（`post_json_in_page`）：在页面上下文里发同源 JSON POST，自动带 `<meta name="csrf-token">` 的 `x-csrf-token`；结果放到 `window.__icpcPost` 轮询取回。
- 读取结果：CF 用 `user.status`；AtCoder 用 kenkoooo `from_second`；洛谷优先 `/record/{rid}?_contentOnly=1`，无 rid 时退回 `/record/list?user_id=&page=1&_contentOnly=1`。

## 洛谷题面

题面不能抓渲染后的页面：洛谷是 Vue SPA，`document.body.innerText` 会把导航、按钮、样例框全部串成一段纯文本，公式与结构全丢。题面取自 `GET /problem/{pid}?_contentOnly=1`：

| JSON 路径 | 含义 |
|---|---|
| `currentData.problem.content.background` | 题目背景 |
| `…content.description` | 题目描述 |
| `…content.formatI` | 输入格式（旧导出叫 `inputFormat`） |
| `…content.formatO` | 输出格式（旧导出叫 `outputFormat`） |
| `…content.hint` | 说明/提示 |
| `problem.samples` | `[[输入, 输出], …]` |
| `problem.limits.time / .memory` | 毫秒 / KiB（128000 → 125 MB） |
| `problem.tags` | 数字 id，需要用 `/_lfe/tags` 换成名字 |
| `problem.difficulty` | 1–7，映射到 800–2800 |

- 各字段原样是 Markdown（含 `$…$`、`$$…$$`），直接拼接成 `## 题目背景` 等小节后入库；行内 `<img>` 转成 `![](url)`、`<br>` 转成换行，其余 HTML 保持原样由渲染器忽略。
- 公式由 `src/lib/markdown.tsx` 的 remark-math + rehype-katex 渲染（`rehypeSanitize` 必须在 `rehypeKatex` 之前，否则 KaTeX 生成的样式会被清掉）。
- 已经存成"整页文本"的旧题面会被识别（含 2 个以上 `个人中心/题目列表/提交答案/加入题单/关于洛谷` 标记，或标签还是 `tag-<id>`），打开题目时自动重取；重取失败则保留旧内容，不会把题面变成错误页。
- 题目页的"重新获取"按钮走 `get_problem(refresh: true)`，强制重新解析。

## 洛谷提交

洛谷没有单独的提交页（`/problem/submit/{pid}` 返回 404），提交编辑器挂在题目页的弹窗里并且是异步挂载的，脚本填表经常在编辑器出现之前执行。自动提交因此改走站内自己的接口：

1. 打开题目页（已登录），从 `<meta name="csrf-token">` 取 CSRF。
2. `POST /fe/api/problem/submit/{pid}`，`content-type: application/json`，body 为 `{code, lang, enableO2: 1}`。
3. 成功响应只有 `rid`；失败可能是 `{status, errorMessage}`、`{code, message}`、`{errorType, ...}` 或一段登录页 HTML，都要当成失败信息回显。
4. 按 `rid` 轮询 `/record/{rid}?_contentOnly=1`，`status >= 2` 即为终态（12=AC、6=WA、5=TLE、4=MLE、3=OLE、7=RE、2=CE、0/1=评测中）。

`lang` 取自洛谷 `/_lfe/config` 的 `CodeLanguage`，当前值：3=C++98、4=C++11、11=C++14、12=C++17、27=C++20、34=C++23；本应用把编辑器的 `-std` 映射到对应 id，缺省 C++17。

未勾选“自动提交”时仍走 DOM 填表（弹窗里的编辑器），填表失败再交给用户手动粘贴；填表会重试到编辑器出现，而不是固定等待。

## 反爬与人工接管

满足任一条件即 `NeedsHuman`：验证码/Cloudflare/Turnstile、登录失效、403、关键选择器超时、提交后长时间无判定。前端展示页面，用户处理后可：
- 重新验证登录；
- 点击“我已提交，读取结果”；
- 或“手动录入提交结果”。

## 新增平台

1. 在 `PLATFORMS` 注册 id/名称/登录 URL/域名白名单（`automation.rs`）。
2. 在 `platforms.rs` 实现同步、题面、样例解析。
3. 在 `submission.rs` 增加提交页 URL、填表/点击脚本、判定轮询；如果平台有自己的提交接口，优先用它而不是 DOM 填表。
4. 在 `agent.rs` 的平台 enum 中加入（如有工具参数）。
5. 更新 `models.rs` 的 `PlatformDto` 能力、前端图标/名称映射。
6. 加 fixtures 与手动 E2E 清单。
