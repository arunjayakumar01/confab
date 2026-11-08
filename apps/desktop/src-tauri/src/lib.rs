//! Confab desktop core. Owns every Google call, the tokens, the SQLite cache,
//! the outbox, the relay socket and the glass window, once per signed-in account
//! (`accounts`). The web view only ever sees the Tauri commands in `commands` and
//! the `relay` / `shell` events of the active account.

pub mod accounts;
pub mod auth;
pub mod cache;
pub mod commands;
pub mod google;
mod huddle_window;
pub mod media;
pub mod outbox;
pub mod profile;
pub mod relay;
pub mod shell;
pub mod unfurl;

use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, Manager};

pub struct AppState {
    pub profile: profile::Profile,
    pub http: reqwest::Client,
    pub auth: Arc<auth::Auth>,
    pub google: Arc<google::Google>,
    pub cache: Arc<cache::Cache>,
    pub relay: relay::RelayManager,
}

impl AppState {
    /// Everything for one profile (`name` is its directory under the app data dir).
    pub fn new(app_data_dir: std::path::PathBuf, name: &str) -> Result<Self, String> {
        let profile = profile::Profile::load(app_data_dir, name)?;
        tracing::info!(
            "profile {} in {} (chat api {}, relay {})",
            profile.name,
            profile.dir.display(),
            profile.endpoints.chat_api,
            profile.endpoints.relay_ws
        );
        let http = reqwest::Client::builder()
            .user_agent(relay::CLIENT_VERSION)
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        let store = auth::TokenStore::for_profile(&profile);
        let auth = Arc::new(auth::Auth::new(
            profile.endpoints.clone(),
            store,
            http.clone(),
            Some(profile.account_file()),
        ));
        let google = Arc::new(google::Google::new(
            http.clone(),
            auth.clone(),
            &profile.endpoints.chat_api,
            &profile.endpoints.people_api,
            &profile.endpoints.tasks_api,
            &profile.endpoints.meet_api,
            &profile.endpoints.calendar_api,
            &profile.endpoints.drive_api,
        ));
        let cache = Arc::new(cache::Cache::open(&profile.db_path())?);
        Ok(Self {
            profile,
            http,
            auth,
            google,
            cache,
            relay: relay::RelayManager::new(),
        })
    }
}

/// Glass: Mica (Windows 11) → Acrylic (older Windows), HUD vibrancy on macOS,
/// nothing on Linux. Never fails startup.
fn apply_glass(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
        if let Err(e) = apply_vibrancy(
            window,
            NSVisualEffectMaterial::HudWindow,
            Some(NSVisualEffectState::Active),
            Some(12.0),
        ) {
            tracing::warn!("vibrancy unavailable: {e}");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Err(mica) = window_vibrancy::apply_mica(window, None) {
            if let Err(acrylic) = window_vibrancy::apply_acrylic(window, Some((18, 18, 18, 125))) {
                tracing::warn!("mica ({mica}) and acrylic ({acrylic}) unavailable");
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = window;
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,prism_desktop_lib=debug"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());
    media::register(builder)
        .setup(|app| {
            let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
            let accounts = Arc::new(accounts::Accounts::boot(data_dir)?);
            app.manage(accounts.clone());
            let state = accounts.active();
            if let Some(window) = app.get_webview_window("main") {
                apply_glass(&window);
            }
            if !state.auth.is_signed_in() && std::env::var("PRISM_TWIN_USER").is_ok() {
                // note: dev-only auto sign-in so scripted runs start signed in
                let auth = state.auth.clone();
                match tauri::async_runtime::block_on(async move {
                    tokio::time::timeout(std::time::Duration::from_secs(15), auth.start()).await
                }) {
                    Ok(Ok(_)) => tracing::info!("auto sign-in done"),
                    Ok(Err(e)) => tracing::warn!("auto sign-in failed: {e}"),
                    Err(_) => tracing::warn!("auto sign-in timed out"),
                }
            }
            if state.auth.is_signed_in() {
                if let Some(u) = state.auth.status().user {
                    match state.cache.ensure_owner(&u.name) {
                        Ok(true) => tracing::info!("cache reset for {}", u.name),
                        Ok(false) => {}
                        Err(e) => tracing::warn!("cache owner check failed: {e}"),
                    }
                }
                state.relay.start(app.handle().clone(), state.clone());
                outbox::resume_unsent(app.handle().clone(), state.clone());
            }
            accounts::boot_others(app.handle().clone(), accounts);
            Ok(())
        })
        // Voice huddles capture the microphone in the main webview; the Meet mini window also gets the camera.
        // Everywhere else the camera stays on Default (a prompt, never silent).
        .on_permission_request(|webview, kind| match kind {
            tauri::webview::PermissionKind::Microphone => tauri::webview::PermissionResponse::Allow,
            tauri::webview::PermissionKind::Camera if huddle_window::is_huddle(webview.label()) => {
                tauri::webview::PermissionResponse::Allow
            }
            _ => tauri::webview::PermissionResponse::Default,
        })
        .on_window_event(|window, event| match event {
            // The mini window closed by hand ends the huddle; a close Confab asks for destroys it without this event.
            tauri::WindowEvent::CloseRequested { .. }
                if huddle_window::is_huddle(window.label()) =>
            {
                let _ = window
                    .app_handle()
                    .emit("shell", &shell::ShellEvent::HuddleWindowClosed);
            }
            tauri::WindowEvent::Destroyed if window.label() == "main" => {
                huddle_window::close(window.app_handle());
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::huddle_window_open,
            commands::huddle_window_close,
            commands::auth_status,
            commands::auth_start,
            commands::sign_out,
            commands::list_spaces,
            commands::list_members,
            commands::search_people,
            commands::setup_dm,
            commands::setup_group,
            commands::create_space,
            commands::list_messages,
            commands::send_message,
            commands::edit_message,
            commands::delete_message,
            commands::toggle_reaction,
            commands::mark_read,
            commands::search,
            commands::upload_attachment,
            commands::upload_attachment_bytes,
            commands::get_attachment,
            commands::open_url,
            commands::set_badge,
            commands::set_zoom,
            commands::unfurl,
            commands::add_members,
            commands::leave_space,
            commands::save_attachment,
            commands::list_tasks,
            commands::add_task,
            commands::set_task_done,
            commands::delete_task,
            commands::create_meet,
            commands::meet_status,
            commands::post_huddle,
            commands::availability_get,
            commands::availability_mark,
            commands::availability_status,
            commands::calendar_list,
            commands::calendar_events,
            commands::calendar_create_event,
            commands::calendar_update_event,
            commands::calendar_respond,
            commands::calendar_delete_event,
            commands::notes_load,
            commands::notes_save,
            commands::drive_status,
            commands::drive_connect,
            commands::drive_list,
            commands::drive_drives,
            commands::drive_create_folder,
            commands::drive_upload,
            commands::drive_upload_bytes,
            commands::drive_update,
            commands::drive_content_url,
            commands::notify,
            commands::relay_send,
            commands::accounts_list,
            commands::account_switch,
            commands::account_add,
            commands::account_remove,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Confab desktop shell");
}
