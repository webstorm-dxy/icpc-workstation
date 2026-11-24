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
            let logical =
                tauri::LogicalSize::new(size.width as f64 / scale, size.height as f64 / scale);
            window.add_child(
                tauri::webview::WebviewBuilder::new(
                    "ui",
                    tauri::WebviewUrl::App("index.html".into()),
                )
                .auto_resize(),
                tauri::LogicalPosition::new(0.0, 0.0),
                logical,
            )?;
            window.show()?;
            window.set_focus()?;
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
            commands::check_platform_handle,
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
