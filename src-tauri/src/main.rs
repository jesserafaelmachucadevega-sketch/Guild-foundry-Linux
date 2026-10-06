// Guild Foundry AI — Tauri 2 native backend.
//
// Architecture rule (master spec section 0): the Rust layer owns ALL privileged
// operations — filesystem, processes, credentials, notifications, tray, window
// management, subprocesses, terminal, local database, OS detection, permissions.
// The web frontend owns presentation and application state. The frontend NEVER
// receives unrestricted filesystem/shell/credential/process capabilities.
//
// Phase modules land in build order (CHECKLIST.md). Only Phase 1 modules are
// declared here so far: db, os_info, settings.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agents;
mod build;
mod db;
mod diagnostics;
mod git;
mod index;
mod mcp;
mod memory;
mod os_info;
mod providers;
mod research;
mod secrets;
mod security;
mod settings;
mod terminal;
mod tools;
mod updater;
mod workspace;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

#[tauri::command]
fn window_set_compact(window: tauri::WebviewWindow, compact: bool) -> Result<(), String> {
    use tauri::dpi::PhysicalSize;
    let size = if compact {
        PhysicalSize { width: 480, height: 800 }
    } else {
        PhysicalSize { width: 1440, height: 900 }
    };
    window
        .set_size(tauri::Size::Physical(size))
        .map_err(|e| e.to_string())
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    // Tray icon is optional: if the bundled icon is missing (e.g. icons were
    // never generated with `tauri icon`), the app still runs without a tray.
    let icon_path = app
        .path()
        .resource_dir()
        .map(|d| d.join("icons").join("tray-32.png"))
        .unwrap_or_default();
    if !icon_path.exists() {
        eprintln!("tray icon not found at {:?}; running without tray", icon_path);
        return Ok(());
    }
    let icon = tauri::image::Image::from_path(&icon_path)?;

    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let _tray = TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Guild Foundry AI")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { .. } = event {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    let _ = if w.is_visible().unwrap_or(true) {
                        w.hide()
                    } else {
                        w.show()
                    };
                }
            }
        })
        .build(app)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_sql::Builder::default().build())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // Phase 1: initialize SQLite (runs migrations) before any command.
            match db::init_db(app.handle()) {
                Ok(path) => println!("database ready at {:?}", path),
                Err(e) => eprintln!("database init failed: {}", e),
            }
            if let Err(e) = build_tray(app.handle()) {
                eprintln!("tray setup failed: {}", e);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            window_set_compact,
            os_info::detect_environment,
            settings::settings_save_prompts,
            settings::settings_load_prompts,
            settings::settings_set,
            settings::settings_get,
            // Phase 2 — providers + secrets
            providers::providers_any_connected,
            providers::provider_list,
            providers::provider_upsert,
            providers::provider_delete,
            providers::provider_handshake,
            providers::provider_list_models,
            providers::provider_refresh_models,
            providers::provider_chat_complete,
            providers::provider_chat_stream,
            providers::provider_chat_cancel,
            providers::provider_router_suggest,
            secrets::secret_set,
            secrets::secret_get,
            secrets::secret_delete,
            secrets::secret_list_keys,
            // Phase 4 — workspace, terminal, git
            workspace::ws_project_root,
            workspace::ws_list,
            workspace::ws_read,
            workspace::ws_write,
            workspace::ws_create,
            workspace::ws_delete,
            workspace::ws_rename,
            workspace::ws_move,
            workspace::ws_search,
            workspace::ws_diff,
            workspace::ws_patch,
            workspace::ws_snapshot,
            workspace::ws_checkpoints,
            workspace::ws_checkpoint_preview,
            workspace::ws_checkpoint_diff,
            workspace::ws_restore,
            terminal::term_exec,
            terminal::term_start,
            terminal::term_output,
            terminal::term_list,
            terminal::term_kill,
            terminal::term_classify,
            git::git_status,
            git::git_diff,
            git::git_log,
            git::git_branch,
            git::git_commit,
            git::git_stash,
            git::git_checkout,
            git::git_merge,
            git::git_remote,
            git::git_pull,
            git::git_push,
            // Phase 5 — agents / builder state machine
            agents::agent_list,
            agents::run_start,
            agents::run_status,
            agents::run_cancel,
            agents::run_transition,
            agents::run_recover,
            agents::run_transitions,
            agents::run_history_list,
            agents::agent_task_create,
            agents::agent_task_update,
            agents::agent_task_get,
            agents::agent_task_list,
            agents::approval_gate_check,
            agents::approval_request,
            agents::approval_resolve,
            agents::approval_list,
            agents::requirements_get,
            agents::requirements_upsert,
            agents::requirements_signoff,
            agents::adr_create,
            agents::adr_list,
            // Phase 6 — tool execution loop
            tools::tool_list,
            tools::tool_handshake,
            tools::tool_execute,
            // Phase 7 — MCP gateway + permissions
            mcp::mcp_server_add,
            mcp::mcp_server_update,
            mcp::mcp_server_remove,
            mcp::mcp_server_list,
            mcp::mcp_server_enable,
            mcp::mcp_server_disable,
            mcp::mcp_server_test,
            mcp::mcp_connect,
            mcp::mcp_list_tools,
            mcp::mcp_list_resources,
            mcp::mcp_list_prompts,
            mcp::mcp_call_tool,
            mcp::mcp_tool_registry_list,
            mcp::mcp_tool_set_enabled,
            mcp::mcp_oauth_start,
            mcp::mcp_oauth_callback,
            mcp::mcp_oauth_revoke,
            mcp::perm_get,
            mcp::perm_set,
            mcp::perm_list,
            // Phase 8 — build/test/fix engine
            build::build_start,
            build::build_status,
            build::build_cancel,
            build::test_run,
            build::template_list,
            build::template_instantiate,
            build::artifact_create,
            build::artifact_list,
            build::artifact_get,
            build::artifact_verify,
            build::artifact_export,
            build::release_create,
            // Phase 9 — security
            security::constitution_text,
            security::sec_wrap_content,
            security::sec_build_prompt,
            security::sec_audit,
            security::sec_audit_query,
            security::sec_classify_command,
            // Phase 10 — memory, indexing, research
            memory::memory_put,
            memory::memory_get,
            memory::memory_search,
            memory::memory_consolidate,
            memory::memory_synthesize,
            index::index_rebuild,
            index::index_search,
            index::index_watch_start,
            index::index_watch_stop,
            research::research_start,
            research::research_status,
            // Phase 11 — updater, diagnostics, crash recovery
            updater::updater_check,
            updater::updater_postpone,
            updater::updater_download_install,
            updater::updater_clear_postpone,
            diagnostics::diag_snapshot,
            diagnostics::diag_network,
            diagnostics::diag_token_stats,
            diagnostics::diag_build_workers,
            diagnostics::crash_state_save,
            diagnostics::crash_state_load,
            diagnostics::crash_state_clear,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Guild Foundry AI");
}
