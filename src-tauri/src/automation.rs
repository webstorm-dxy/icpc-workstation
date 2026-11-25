use crate::error::{AppError, AppResult};
use crate::models::{AutomationBounds, AutomationEvent, AutomationStateDto};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl, Wry};
use tokio::sync::oneshot;
use url::Url;

/// A deliberately small layer around Tauri's multi-webview API. All reads
/// from the page use `eval_with_callback`, so OJ pages never receive a usable
/// Tauri IPC surface.
#[derive(Default)]
pub struct AutomationManager {
    views: Mutex<HashMap<String, Webview<Wry>>>,
    states: Mutex<HashMap<String, AutomationStateDto>>,
}

fn label(platform: &str) -> String {
    format!("oj-{platform}")
}

fn user_agent(platform: &str) -> &'static str {
    match platform {
        // Luogu's WAF behaves better with a mainstream Chrome string.
        "luogu" => {
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
        }
        _ => {
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Safari/605.1.15"
        }
    }
}

fn allowed_host(platform: &str, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    match platform {
        "codeforces" => {
            host == "codeforces.com"
                || host.ends_with(".codeforces.com")
                || host == "accounts.google.com"
                || host == "github.com"
                || host.ends_with(".github.com")
        }
        "atcoder" => {
            host == "atcoder.jp"
                || host.ends_with(".atcoder.jp")
                || host == "accounts.google.com"
                || host == "github.com"
        }
        "luogu" => host == "luogu.com.cn" || host.ends_with(".luogu.com.cn"),
        _ => false,
    }
}

impl AutomationManager {
    pub fn state(&self, platform: &str) -> AutomationStateDto {
        self.states
            .lock()
            .expect("automation state lock")
            .get(platform)
            .cloned()
            .unwrap_or_else(|| AutomationStateDto {
                platform: platform.to_string(),
                visible: false,
                url: None,
                status: "idle".into(),
                message: String::new(),
                needs_human: false,
                hint: None,
            })
    }

    pub fn set_state(&self, app: &AppHandle, state: AutomationStateDto) {
        self.states
            .lock()
            .expect("automation state lock")
            .insert(state.platform.clone(), state.clone());
        let _ = app.emit(
            "automation://state",
            AutomationEvent {
                platform: state.platform,
                status: state.status,
                message: state.message,
                needs_human: state.needs_human,
                hint: state.hint,
                url: state.url,
            },
        );
    }

    pub async fn open(
        &self,
        app: &AppHandle,
        platform: &str,
        url: &str,
        bounds: Option<AutomationBounds>,
    ) -> AppResult<()> {
        let target =
            Url::parse(url).map_err(|e| AppError::new("E_URL", format!("无效的页面地址: {e}")))?;
        if !allowed_host(platform, target.host_str().unwrap_or("")) {
            return Err(AppError::new(
                "E_URL_DENIED",
                format!("地址不在 {} 的允许域名内", platform),
            ));
        }
        let label = label(platform);
        let existing = {
            self.views
                .lock()
                .expect("automation view lock")
                .get(platform)
                .cloned()
        };
        if let Some(view) = existing {
            let _ = view.navigate(target);
            let _ = view.show();
            if let Some(bounds) = bounds {
                apply_bounds(app, &view, &bounds);
            }
            let mut state = self.state(platform);
            state.visible = true;
            state.url = Some(url.to_string());
            state.status = "opening".into();
            state.message = "正在打开页面…".into();
            self.set_state(app, state);
            return Ok(());
        }
        let window = app
            .get_window("main")
            .ok_or_else(|| AppError::new("E_WINDOW", "主窗口不存在"))?;
        let app_for_load = app.clone();
        let platform_for_load = platform.to_string();
        let app_for_nav = app.clone();
        let platform_for_nav = platform.to_string();
        let app_for_new = app.clone();
        let platform_for_new = platform.to_string();
        let builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(target))
            .user_agent(user_agent(platform))
            .incognito(false)
            .on_page_load(move |view, payload| {
                let _ = app_for_load.emit(
                    "automation://page",
                    serde_json::json!({
                        "platform": platform_for_load,
                        "url": payload.url().to_string(),
                        "event": format!("{:?}", payload.event())
                    }),
                );
                if matches!(payload.event(), PageLoadEvent::Finished) {
                    let state = app_for_load.state::<crate::state::SharedState>();
                    let mut current = state.automation.state(&platform_for_load);
                    if current.status == "opening" {
                        current.status = "ready".into();
                        current.message = "页面已加载，可在此登录或操作".into();
                        current.url = Some(payload.url().to_string());
                        state.automation.set_state(&app_for_load, current);
                    }
                }
                let _ = view;
            })
            .on_navigation(move |url| {
                let ok = allowed_host(&platform_for_nav, url.host_str().unwrap_or(""));
                if !ok {
                    let _ = app_for_nav.emit(
                        "automation://external",
                        serde_json::json!({ "platform": platform_for_nav, "url": url.to_string() }),
                    );
                }
                ok
            })
            .on_new_window(move |url, _features| {
                let _ = app_for_new.emit(
                    "automation://external",
                    serde_json::json!({ "platform": platform_for_new, "url": url.to_string() }),
                );
                NewWindowResponse::Deny
            });
        let bounds = bounds.unwrap_or(AutomationBounds {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 600.0,
            viewport_height: None,
        });
        let inset = top_inset(app, &bounds);
        let view = window
            .add_child(
                builder,
                LogicalPosition::new(bounds.x.max(0.0), (bounds.y + inset).max(0.0)),
                LogicalSize::new(bounds.width.max(100.0), bounds.height.max(100.0)),
            )
            .map_err(|e| AppError::new("E_AUTOMATION", format!("无法创建 OJ 页面窗口: {e}")))?;
        self.views
            .lock()
            .expect("automation view lock")
            .insert(platform.to_string(), view);
        self.set_state(
            app,
            AutomationStateDto {
                platform: platform.to_string(),
                visible: true,
                url: Some(url.to_string()),
                status: "opening".into(),
                message: format!("正在打开 {}…", crate::util::platform_label(platform)),
                needs_human: false,
                hint: None,
            },
        );
        Ok(())
    }

    pub fn set_bounds(&self, app: &AppHandle, platform: &str, bounds: &AutomationBounds) {
        if let Some(view) = self
            .views
            .lock()
            .expect("automation view lock")
            .get(platform)
            .cloned()
        {
            apply_bounds(app, &view, bounds);
        }
    }

    pub fn set_visible(&self, platform: &str, visible: bool) {
        if let Some(view) = self
            .views
            .lock()
            .expect("automation view lock")
            .get(platform)
            .cloned()
        {
            if visible {
                let _ = view.show();
            } else {
                let _ = view.hide();
            }
        }
        if let Ok(mut states) = self.states.lock() {
            if let Some(state) = states.get_mut(platform) {
                state.visible = visible;
            }
        }
    }

    pub fn close(&self, platform: &str) {
        let view = self
            .views
            .lock()
            .expect("automation view lock")
            .remove(platform);
        if let Some(view) = view {
            let _ = view.close();
        }
        if let Ok(mut states) = self.states.lock() {
            if let Some(state) = states.get_mut(platform) {
                state.visible = false;
                state.status = "idle".into();
            }
        }
    }

    pub async fn eval_json(
        &self,
        platform: &str,
        script: &str,
        timeout_ms: u64,
    ) -> AppResult<Value> {
        let view = self
            .views
            .lock()
            .expect("automation view lock")
            .get(platform)
            .cloned()
            .ok_or_else(|| AppError::new("E_AUTOMATION", "自动化页面尚未打开"))?;
        let (tx, rx) = oneshot::channel::<String>();
        let tx = Mutex::new(Some(tx));
        view.eval_with_callback(script, move |result| {
            if let Some(tx) = tx.lock().expect("eval callback lock").take() {
                let _ = tx.send(result);
            }
        })
        .map_err(|e| AppError::new("E_AUTOMATION", format!("页面脚本执行失败: {e}")))?;
        let raw = tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx)
            .await
            .map_err(|_| {
                AppError::new("E_AUTOMATION_TIMEOUT", "页面响应超时")
                    .with_detail("OJ 页面可能加载缓慢或触发了人机验证")
                    .retryable()
            })?
            .map_err(|_| AppError::new("E_AUTOMATION", "页面脚本未返回结果"))?;
        decode_eval_result(&raw)
    }

    pub async fn wait_ready(&self, platform: &str, timeout_ms: u64) -> AppResult<()> {
        let js = "JSON.stringify({ready: document.readyState === 'complete'})";
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if let Ok(value) = self.eval_json(platform, js, 2_000).await {
                if value.get("ready").and_then(|v| v.as_bool()) == Some(true) {
                    return Ok(());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::new(
                    "E_AUTOMATION_TIMEOUT",
                    "页面加载超时，可能触发了人机验证",
                )
                .retryable());
            }
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }
    }

    /// Runs an async browser operation that stores its result in
    /// `window.__icpcResult`, then polls for it. Used by Luogu's same-origin
    /// JSON endpoints, which need the page's cookies.
    pub async fn fetch_json_in_page(
        &self,
        platform: &str,
        path: &str,
        timeout_ms: u64,
    ) -> AppResult<Value> {
        let path_json = serde_json::to_string(path).map_err(AppError::internal)?;
        let start = format!(
            r#"(() => {{
                window.__icpcResult = null;
                fetch({path_json}, {{ credentials: 'include', headers: {{ 'x-lentille-request': 'content-only', 'X-Requested-With': 'XMLHttpRequest', 'Accept': 'application/json' }} }})
                  .then(async (r) => {{
                    const text = await r.text();
                    let body;
                    try {{ body = JSON.parse(text); }} catch (_) {{ body = {{ __raw: text.slice(0, 200000) }}; }}
                    window.__icpcResult = {{ status: r.status, body }};
                  }})
                  .catch((e) => {{ window.__icpcResult = {{ status: 0, error: String(e) }}; }});
                return JSON.stringify({{ started: true }});
            }})()"#
        );
        self.eval_json(platform, &start, 5_000).await?;
        self.poll_page_global(
            platform,
            "__icpcResult",
            timeout_ms,
            "等待洛谷数据超时，可能需要完成人机验证",
        )
        .await
    }

    /// Issues a same-origin JSON POST from inside the OJ page.
    ///
    /// Code must be sent by the page itself: only it carries the session cookies
    /// and the `csrf-token` meta value the platform checks on writes. The result
    /// is parked on `window.__icpcPost` and polled, because `eval_with_callback`
    /// cannot await a promise.
    pub async fn post_json_in_page(
        &self,
        platform: &str,
        path: &str,
        body: &str,
        timeout_ms: u64,
    ) -> AppResult<Value> {
        let path_json = serde_json::to_string(path).map_err(AppError::internal)?;
        let body_json = serde_json::to_string(body).map_err(AppError::internal)?;
        let start = format!(
            r#"(() => {{
                window.__icpcPost = null;
                const meta = document.querySelector('meta[name="csrf-token"]');
                const csrf = meta ? meta.getAttribute('content') || '' : '';
                fetch({path_json}, {{
                    method: 'POST',
                    credentials: 'include',
                    headers: {{
                        'content-type': 'application/json',
                        'accept': 'application/json',
                        'x-requested-with': 'XMLHttpRequest',
                        'x-csrf-token': csrf
                    }},
                    body: {body_json}
                }})
                  .then(async (r) => {{
                    const text = await r.text();
                    let parsed;
                    try {{ parsed = JSON.parse(text); }} catch (_) {{ parsed = {{ __raw: text.slice(0, 4000) }}; }}
                    window.__icpcPost = {{ status: r.status, redirected: r.redirected, body: parsed }};
                  }})
                  .catch((e) => {{ window.__icpcPost = {{ status: 0, error: String(e) }}; }});
                return JSON.stringify({{ started: true }});
            }})()"#
        );
        self.eval_json(platform, &start, 5_000).await?;
        self.poll_page_global(
            platform,
            "__icpcPost",
            timeout_ms,
            "等待提交响应超时，请检查页面是否需要验证",
        )
        .await
    }

    async fn poll_page_global(
        &self,
        platform: &str,
        global: &str,
        timeout_ms: u64,
        timeout_message: &str,
    ) -> AppResult<Value> {
        let poll = format!(
            "(() => {{ const r = window.{global}; return r === null || r === undefined ? 'null' : JSON.stringify(r); }})()"
        );
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            match self.eval_json(platform, &poll, 5_000).await {
                Ok(Value::Null) => {}
                Ok(value) => return Ok(value),
                Err(_) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::new("E_AUTOMATION_TIMEOUT", timeout_message).retryable());
            }
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }
    }

    pub async fn set_status(
        &self,
        app: &AppHandle,
        platform: &str,
        status: &str,
        message: impl Into<String>,
    ) {
        let mut state = self.state(platform);
        state.status = status.to_string();
        state.message = message.into();
        self.set_state(app, state);
    }

    pub async fn needs_human(
        &self,
        app: &AppHandle,
        platform: &str,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) {
        let mut state = self.state(platform);
        state.status = "needs_human".into();
        state.message = message.into();
        state.needs_human = true;
        state.hint = Some(hint.into());
        state.visible = true;
        self.set_state(app, state);
        if let Some(view) = self
            .views
            .lock()
            .expect("automation view lock")
            .get(platform)
            .cloned()
        {
            let _ = view.show();
            let _ = view.set_focus();
        }
    }
}

fn top_inset(app: &AppHandle, bounds: &AutomationBounds) -> f64 {
    // The DOM rect is relative to the UI webview's viewport. Native child
    // webviews are placed in window-frame coordinates. On macOS the viewport
    // starts below the title bar while the frame does not, which would make
    // the OJ page cover the automation header if the raw DOM y were used.
    let window = app.get_window("main");
    let scale = window
        .as_ref()
        .and_then(|window| window.scale_factor().ok())
        .unwrap_or(1.0);
    let frame_height = window
        .as_ref()
        .and_then(|window| window.inner_size().ok())
        .map(|size| size.height as f64 / scale)
        .unwrap_or(0.0);
    compute_top_inset(frame_height, bounds.viewport_height)
}

fn compute_top_inset(frame_height: f64, viewport_height: Option<f64>) -> f64 {
    match viewport_height {
        Some(viewport_height) if viewport_height > 0.0 && frame_height > 0.0 => {
            (frame_height - viewport_height).max(0.0)
        }
        _ => 0.0,
    }
}

fn apply_bounds(app: &AppHandle, view: &Webview<Wry>, bounds: &AutomationBounds) {
    let inset = top_inset(app, bounds);
    let _ = view.set_position(LogicalPosition::new(
        bounds.x.max(0.0),
        (bounds.y + inset).max(0.0),
    ));
    let _ = view.set_size(LogicalSize::new(
        bounds.width.max(100.0),
        bounds.height.max(100.0),
    ));
}

fn decode_eval_result(raw: &str) -> AppResult<Value> {
    let outer: Value = serde_json::from_str(raw).unwrap_or(Value::String(raw.to_string()));
    match outer {
        Value::String(inner) => {
            let trimmed = inner.trim();
            if trimmed.is_empty() {
                return Ok(Value::Null);
            }
            serde_json::from_str(trimmed)
                .map_err(|e| AppError::new("E_AUTOMATION", format!("页面返回了非 JSON 数据: {e}")))
        }
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_scoped_per_platform() {
        assert!(allowed_host("codeforces", "codeforces.com"));
        assert!(allowed_host("atcoder", "atcoder.jp"));
        assert!(allowed_host("luogu", "www.luogu.com.cn"));
        assert!(!allowed_host("luogu", "codeforces.com"));
        assert!(!allowed_host("atcoder", "evil-atcoder.jp"));
    }

    #[test]
    fn computes_title_bar_inset() {
        assert!((compute_top_inset(870.0, Some(842.0)) - 28.0).abs() < f64::EPSILON);
        assert_eq!(compute_top_inset(842.0, Some(842.0)), 0.0);
        assert_eq!(compute_top_inset(870.0, None), 0.0);
    }

    #[test]
    fn decodes_double_encoded_results() {
        let value = decode_eval_result("\"{\\\"ok\\\":true}\"").unwrap();
        assert_eq!(value["ok"], true);
        let value = decode_eval_result("{\"ok\":false}").unwrap();
        assert_eq!(value["ok"], false);
    }
}
