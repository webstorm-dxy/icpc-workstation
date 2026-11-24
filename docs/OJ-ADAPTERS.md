# OJ 适配器

## 能力矩阵

| 平台 | 登录 | 读取 | 题面/样例 | 提交 | 反爬 |
|---|---|---|---|---|---|
| Codeforces | `/enter` | 官方 API | HTML（`.problem-statement`、`.sample-test`） | 内置浏览器 + API 轮询 | Cloudflare 偶发 |
| AtCoder | `/login` | kenkoooo API + `/users/{handle}/history/json` | HTML（`#task-statement`、`pre`） | 内置浏览器 + kenkoooo 轮询 | 提交冷却 |
| 洛谷 | `/auth/login` | 页面内 same-origin `fetch`（`_contentOnly=1`） | `_contentOnly` JSON；DOM 兜底 | 内置浏览器 | WAF/验证码频繁 |

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
- 填表脚本：设置 problem/task select、language select、编辑器（CodeMirror 5 / Ace / Monaco / textarea），返回 `{ok, language, editor}`。
- 点击脚本：按文字/类型找提交按钮。
- 读取结果：CF 用 `user.status`；AtCoder 用 kenkoooo `from_second`；洛谷用 `/record/list?user_id=&page=1&_contentOnly=1`。

## 反爬与人工接管

满足任一条件即 `NeedsHuman`：验证码/Cloudflare/Turnstile、登录失效、403、关键选择器超时、提交后长时间无判定。前端展示页面，用户处理后可：
- 重新验证登录；
- 点击“我已提交，读取结果”；
- 或“手动录入提交结果”。

## 新增平台

1. 在 `PLATFORMS` 注册 id/名称/登录 URL/域名白名单（`automation.rs`）。
2. 在 `platforms.rs` 实现同步、题面、样例解析。
3. 在 `submission.rs` 增加提交页 URL、填表/点击脚本、判定轮询。
4. 在 `agent.rs` 的平台 enum 中加入（如有工具参数）。
5. 更新 `models.rs` 的 `PlatformDto` 能力、前端图标/名称映射。
6. 加 fixtures 与手动 E2E 清单。
