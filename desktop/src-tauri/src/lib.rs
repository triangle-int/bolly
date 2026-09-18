mod computer_use;
mod computer_use_bridge;
mod overlay;
mod permissions;
mod screen_recorder;

use std::sync::{Arc, Mutex};

use tauri::menu::{AboutMetadataBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::webview::WebviewWindowBuilder;
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_opener::OpenerExt;

/// Tracks the host of the currently connected server so on_navigation can allow it.
struct ServerOrigin(Arc<Mutex<Option<String>>>);

/// Returns true if the URL is an internal app URL that should stay in the webview.
fn is_internal_url(url: &url::Url) -> bool {
    match url.scheme() {
        "tauri" => true,
        "http" | "https" => {
            let host = url.host_str().unwrap_or("");
            host == "localhost" || host == "tauri.localhost"
        }
        _ => false,
    }
}

#[tauri::command]
fn navigate(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let ww = app
        .get_webview_window("main")
        .ok_or("main webview not found")?;
    let parsed: url::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    // Register the server host so on_navigation allows subsequent navigations
    if let Some(host) = parsed.host_str() {
        let state = app.state::<ServerOrigin>();
        *state.0.lock().unwrap() = Some(host.to_string());
    }
    ww.navigate(parsed).map_err(|e| e.to_string())
}

fn navigate_home(app: &tauri::AppHandle) -> Result<(), String> {
    // Clear the server origin — we're going back to the dashboard
    let state = app.state::<ServerOrigin>();
    *state.0.lock().unwrap() = None;

    let ww = app
        .get_webview_window("main")
        .ok_or("main webview not found")?;
    let url = if cfg!(debug_assertions) {
        "http://localhost:1420/"
    } else {
        "tauri://localhost/"
    };
    let parsed: url::Url = url.parse().map_err(|e: url::ParseError| e.to_string())?;
    ww.navigate(parsed).map_err(|e| e.to_string())
}

fn settings_url() -> String {
    if cfg!(debug_assertions) {
        "http://localhost:1420/settings/".to_string()
    } else {
        "tauri://localhost/settings/".to_string()
    }
}

fn open_settings_window(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::webview::WebviewWindowBuilder;

    // If settings window already exists, just focus it
    if let Some(win) = app.get_webview_window("settings") {
        win.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    let url = settings_url();
    let nav_handle = app.clone();
    WebviewWindowBuilder::new(
        app,
        "settings",
        tauri::WebviewUrl::External(url.parse().map_err(|e: url::ParseError| e.to_string())?),
    )
    .title("Bolly Settings")
    .inner_size(420.0, 480.0)
    .resizable(false)
    .center()
    .on_navigation(move |url| {
        if is_internal_url(url) {
            return true;
        }
        let _ = nav_handle.opener().open_url(url.as_str(), None::<&str>);
        false
    })
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_store::Builder::default().build());

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    builder
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            navigate,
            computer_use::computer_screenshot,
            computer_use::computer_click,
            computer_use::computer_double_click,
            computer_use::computer_mouse_move,
            computer_use::computer_scroll,
            computer_use::computer_type,
            computer_use::computer_key,
            computer_use_bridge::connect_computer_use,
            computer_use_bridge::disconnect_computer_use,
            computer_use_bridge::set_screen_recording_allowed,
            computer_use_bridge::get_screen_recording_allowed,
            computer_use_bridge::get_server_url,
            computer_use_bridge::stop_screen_recording,
            computer_use_bridge::set_instance_slug,
            permissions::check_permissions,
            permissions::open_permission_settings,
        ])
        .setup(|app| {
            // Shared state: the host of the connected server (if any)
            let origin: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
            app.manage(ServerOrigin(origin.clone()));

            // Single instance must be registered first in setup
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                app.handle().plugin(tauri_plugin_single_instance::init(
                    move |_app, argv, _cwd| {
                        // Focus existing window
                        if let Some(win) = handle.get_webview_window("main") {
                            win.set_focus().ok();
                        }
                        // Forward deep link URL
                        for arg in &argv {
                            if arg.starts_with("bolly://") {
                                handle.emit("deep-link", arg.clone()).ok();
                            }
                        }
                    },
                ))?;
            }
            let about = AboutMetadataBuilder::new()
                .name(Some("Bolly"))
                .version(Some(env!("CARGO_PKG_VERSION")))
                .website(Some("https://bollyai.dev"))
                .website_label(Some("bollyai.dev"))
                .comments(Some("Your AI companion"))
                .build();

            let settings = MenuItemBuilder::with_id("settings", "Settings...")
                .accelerator("CmdOrCtrl+,")
                .build(app)?;

            let app_menu = SubmenuBuilder::new(app, "Bolly")
                .about(Some(about))
                .separator()
                .items(&[&settings])
                .separator()
                .hide()
                .hide_others()
                .show_all()
                .separator()
                .quit()
                .build()?;

            let edit_menu = SubmenuBuilder::new(app, "Edit")
                .undo()
                .redo()
                .separator()
                .cut()
                .copy()
                .paste()
                .select_all()
                .build()?;

            let back = MenuItemBuilder::with_id("back", "Back to Dashboard")
                .accelerator("CmdOrCtrl+Shift+D")
                .build(app)?;

            let view_menu = SubmenuBuilder::new(app, "View").items(&[&back]).build()?;

            let menu = MenuBuilder::new(app)
                .items(&[&app_menu, &edit_menu, &view_menu])
                .build()?;
            app.set_menu(menu)?;

            let handle = app.handle().clone();
            app.on_menu_event(move |_app, event| {
                if event.id() == "back" {
                    let _ = navigate_home(&handle);
                } else if event.id() == "settings" {
                    let _ = open_settings_window(&handle);
                }
            });

            // Create main window programmatically so we can attach on_navigation
            let nav_handle = app.handle().clone();
            let origin_for_nav = origin.clone();
            WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
                .title("Bolly")
                .inner_size(1024.0, 700.0)
                .min_inner_size(480.0, 400.0)
                .center()
                .resizable(true)
                .disable_drag_drop_handler()
                .on_navigation(move |url| {
                    if is_internal_url(url) {
                        return true;
                    }
                    // Allow navigation to the connected server
                    if let Some(host) = url.host_str() {
                        if let Some(ref server) = *origin_for_nav.lock().unwrap() {
                            if host == server {
                                return true;
                            }
                        }
                    }
                    let _ = nav_handle.opener().open_url(url.as_str(), None::<&str>);
                    false
                })
                .build()?;

            // Handle deep links received while app is running
            let handle2 = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                if let Some(url) = event.urls().first() {
                    handle2.emit("deep-link", url.to_string()).ok();
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
