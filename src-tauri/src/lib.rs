mod agent;
mod analysis;
mod automation;
mod commands;
mod db;
mod error;
mod judge;
mod llm;
mod models;
mod plan;
mod platforms;
mod provider;
mod reports;
mod secrets;
mod state;
mod submission;
mod util;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = tauri::async_runtime::block_on(state::init(app.handle()))?;
            // Dev/demo escape hatch: populate a realistic workspace on first launch.
            if std::env::var("ICPC_AUTOSEED").ok().as_deref() == Some("1") {
                let complete =
                    tauri::async_runtime::block_on(state.db.get_setting("onboarding.complete"));
                if complete.as_deref() != Some("true") {
                    if let Err(err) =
                        tauri::async_runtime::block_on(commands::seed_demo(app.handle(), &state))
                    {
                        eprintln!("demo seed failed: {err}");
                    }
                }
            }
            let state_for_resume = state.clone();
            let app_for_resume = app.handle().clone();
            app.manage(state);
            tauri::async_runtime::spawn(async move {
                analysis::resume_pending(
                    &app_for_resume,
                    &state_for_resume.db,
                    &state_for_resume.llm,
                )
                .await;
            });

            let window = tauri::window::WindowBuilder::new(app, "main")
                .title("ICPC 备赛工作站")
                .inner_size(1440.0, 900.0)
                .min_inner_size(1100.0, 720.0)
                .build()?;
            let size = window.inner_size()?;
            let scale = window.scale_factor()?;
            let logical = tauri::LogicalSize::new(
                size.width as f64 / scale,
                size.height as f64 / scale,
            );
            window.add_child(
                tauri::webview::WebviewBuilder::new("ui", tauri::WebviewUrl::App("index.html".into()))
                    .auto_resize()
                    .initialization_script(
                        r#"window.__ICPC_ERRORS__ = [];
(function () {
  var push = function (v) { try { window.__ICPC_ERRORS__.push(String(v)); } catch (_) {} };
  window.addEventListener('error', function (event) { push(event.message || 'error'); });
  window.addEventListener('unhandledrejection', function (event) { push(event.reason || 'rejection'); });
  window.addEventListener('securitypolicyviolation', function (event) { push('CSP: ' + event.violatedDirective + ' ' + event.blockedURI); });
  var origError = console.error;
  console.error = function () { push(Array.prototype.map.call(arguments, String).join(' ')); return origError.apply(console, arguments); };
})();"#,
                    ),
                tauri::LogicalPosition::new(0.0, 0.0),
                logical,
            )?;
            window.show()?;
            window.set_focus()?;
            if let Ok(test_url) = std::env::var("ICPC_TEST_OPEN_URL") {
                let app_open = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let state = app_open.state::<state::SharedState>().inner().clone();
                    let bounds = models::AutomationBounds {
                        x: 40.0,
                        y: 80.0,
                        width: 720.0,
                        height: 620.0,
                    };
                    let opened = state
                        .automation
                        .open(&app_open, "codeforces", &test_url, Some(bounds))
                        .await;
                    let mut out = serde_json::json!({
                        "openOk": opened.is_ok(),
                        "openError": opened.err().map(|e| e.message),
                    });
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    if let Ok(value) = state
                        .automation
                        .eval_json(
                            "codeforces",
                            "({title: document.title, url: location.href, ready: document.readyState, len: (document.body && document.body.innerText || '').length})",
                            5_000,
                        )
                        .await
                    {
                        out["page"] = value;
                    }
                    if let Ok(path) = std::env::var("ICPC_TEST_OPEN_DUMP") {
                        let _ = std::fs::write(path, out.to_string());
                    }
                    state.automation.close("codeforces");
                });
            }
            if let Some(dump_path) = std::env::var("ICPC_DUMP_UI").ok().filter(|v| !v.is_empty()) {
                let app_dump = app.handle().clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        std::thread::sleep(std::time::Duration::from_millis(800));
                        if let Some(webview) = app_dump.get_webview("ui") {
                            let (tx, rx) = std::sync::mpsc::channel::<String>();
                            let tx = std::sync::Mutex::new(Some(tx));
                            let js = r#"({
                                url: location.href,
                                hash: location.hash,
                                title: document.title,
                                text: (document.body && document.body.innerText || '').slice(0, 40000),
                                htmlLen: document.body ? document.body.innerHTML.length : 0,
                                rootLen: (document.getElementById('root') && document.getElementById('root').innerHTML.length) || 0,
                                cards: document.querySelectorAll('.ant-card').length,
                                buttons: document.querySelectorAll('button').length,
                                rows: document.querySelectorAll('tr').length,
                                scripts: document.querySelectorAll('script').length,
                                errors: window.__ICPC_ERRORS__ || []
                            })"#;
                            let ok = webview
                                .eval_with_callback(js, move |result| {
                                    if let Some(tx) = tx.lock().expect("dump lock").take() {
                                        let _ = tx.send(result);
                                    }
                                })
                                .is_ok();
                            if ok {
                                if let Ok(result) = rx.recv_timeout(std::time::Duration::from_secs(2)) {
                                    let _ = std::fs::write(&dump_path, &result);
                                    let text_len = serde_json::from_str::<serde_json::Value>(&result)
                                        .ok()
                                        .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(|t| t.len()))
                                        .unwrap_or(0);
                                    if text_len > 0 {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                });
            }
            if let Ok(route) = std::env::var("ICPC_START_ROUTE") {
                let app_route = app.handle().clone();
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        std::thread::sleep(std::time::Duration::from_millis(700));
                        if let Some(webview) = app_route.get_webview("ui") {
                            let _ = webview.eval(format!(
                                "window.location.hash = '#{}';",
                                route.replace('\'', "")
                            ));
                        }
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_onboarding_state,
            commands::set_onboarding_step,
            commands::set_onboarding_platforms,
            commands::complete_onboarding,
            commands::get_settings,
            commands::update_settings,
            commands::reset_data,
            commands::get_app_info,
            commands::list_providers,
            commands::upsert_provider,
            commands::delete_provider,
            commands::test_provider,
            commands::list_models,
            commands::get_provider_usage,
            commands::get_role_models,
            commands::set_role_model,
            commands::list_platforms,
            commands::save_platform_account,
            commands::open_platform_login,
            commands::open_automation,
            commands::verify_platform_login,
            commands::set_automation_bounds,
            commands::set_automation_visible,
            commands::close_automation,
            commands::get_automation_state,
            commands::start_sync,
            commands::get_dashboard,
            commands::get_problem,
            commands::get_draft,
            commands::save_draft,
            commands::run_samples,
            commands::detect_toolchains,
            commands::generate_plan,
            commands::get_active_plan,
            commands::set_plan_item_status,
            commands::submit_solution,
            commands::capture_verdict,
            commands::record_manual_submission,
            commands::get_submission,
            commands::list_submissions,
            commands::get_analysis,
            commands::reanalyze_submission,
            commands::generate_report,
            commands::list_reports,
            commands::get_report,
            commands::delete_report,
            commands::export_report,
            commands::get_print_report_id,
            commands::print_report_window,
            commands::close_print_window,
            commands::list_conversations,
            commands::create_conversation,
            commands::delete_conversation,
            commands::get_messages,
            commands::send_message,
            commands::cancel_message,
            commands::confirm_tool,
            commands::seed_demo_data,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
