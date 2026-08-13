mod commands;
mod secrets;
mod state;

use std::sync::Arc;

use cockpit_core::Storage;
use tauri::Manager;
use tauri_plugin_log::{Target, TargetKind};

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::LogDir {
                        file_name: Some("cockpit".into()),
                    }),
                    Target::new(TargetKind::Stdout),
                ])
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let log_dir = app.path().app_log_dir()?;
            let storage =
                Storage::open(data_dir.join("cockpit.db")).map_err(|error| error.to_string())?;
            let state =
                AppState::new(storage, &data_dir, log_dir).map_err(|error| error.to_string())?;
            app.manage(Arc::new(state));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::has_connection_password,
            commands::save_connection,
            commands::delete_connection,
            commands::test_connection,
            commands::connect_connection,
            commands::open_tab_session,
            commands::close_tab_session,
            commands::disconnect_connection,
            commands::connect_redis_connection,
            commands::disconnect_redis_connection,
            commands::list_redis_databases,
            commands::scan_redis_keys,
            commands::get_redis_key_info,
            commands::get_redis_value,
            commands::set_redis_string,
            commands::delete_redis_keys,
            commands::expire_redis_key,
            commands::rename_redis_key,
            commands::run_redis_command,
            commands::get_redis_server_info,
            commands::list_databases,
            commands::list_tables,
            commands::list_columns,
            commands::get_table_detail,
            commands::list_routines,
            commands::list_triggers,
            commands::list_events,
            commands::get_object_definition,
            commands::get_routine_parameters,
            commands::list_server_processes,
            commands::kill_server_process,
            commands::get_server_status,
            commands::list_server_variables,
            commands::list_server_locks,
            commands::list_database_users,
            commands::get_user_grants,
            commands::assess_query,
            commands::execute_query,
            commands::load_workspace_state,
            commands::save_workspace_state,
            commands::read_text_file,
            commands::write_text_file,
            commands::reveal_file,
            commands::write_binary_file,
            commands::mutate_row,
            commands::begin_transaction,
            commands::commit_transaction,
            commands::rollback_transaction,
            commands::transaction_active,
            commands::cancel_query,
            commands::export_result_page,
            commands::export_table,
            commands::export_query,
            commands::backup_database,
            commands::preview_import_data,
            commands::import_data,
            commands::cancel_transfer,
            commands::get_runtime_stats,
            commands::get_diagnostics,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Cockpit");
}
