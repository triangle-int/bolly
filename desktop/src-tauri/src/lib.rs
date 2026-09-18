mod companion_relay;
mod computer_use;
mod computer_use_bridge;
mod credentials;
mod overlay;
mod permissions;
mod screen_recorder;

use std::sync::Mutex;

use tauri::menu::{AboutMetadataBuilder, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::webview::WebviewWindowBuilder;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

struct CompanionRelay(Mutex<Option<companion_relay::Relay>>);

/// Returns true if the URL is an internal app URL that should stay in the webview.
fn is_internal_url(url: &url::Url) -> bool {
    match url.scheme() {
        "tauri" => true,
        "http" | "https" => {
            url.origin().ascii_serialization() == "http://tauri.localhost"
                || url.origin().ascii_serialization() == "https://tauri.localhost"
                || (cfg!(debug_assertions)
                    && url.origin().ascii_serialization() == "http://localhost:1420")
        }
        _ => false,
    }
}

fn companion_allows_navigation(url: &url::Url, origin: &str) -> bool {
    url.origin().ascii_serialization() == origin
}

async fn navigate(app: tauri::AppHandle, url: String, auth_token: String) -> Result<(), String> {
    let parsed = connection_url(&url)?;
    close_companion(&app)?;
    let relay = companion_relay::start(parsed, auth_token).await?;
    let origin = relay.origin.clone();
    let back_handle = app.clone();
    let closed_origin = relay.origin.clone();
    let window = WebviewWindowBuilder::new(
        &app,
        "companion",
        tauri::WebviewUrl::External(
            relay
                .origin
                .parse()
                .map_err(|_| "Invalid companion origin")?,
        ),
    )
    .title("Bolly")
    .inner_size(1024.0, 700.0)
    .min_inner_size(480.0, 400.0)
    .incognito(true)
    .disable_drag_drop_handler()
    .initialization_script(&relay.script)
    .on_navigation(move |url| companion_allows_navigation(url, &origin))
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .build()
    .map_err(|_| "Could not open companion window")?;
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            let state = back_handle.state::<CompanionRelay>();
            let mut active = state.0.lock().unwrap();
            let show_dashboard = active
                .as_ref()
                .is_none_or(|relay| relay.origin == closed_origin);
            if show_dashboard {
                active.take();
            }
            drop(active);
            if show_dashboard {
                if let Some(main) = back_handle.get_webview_window("main") {
                    let _ = main.show();
                }
            }
        }
    });
    *app.state::<CompanionRelay>().0.lock().unwrap() = Some(relay);
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.hide();
    }
    Ok(())
}

fn close_companion(app: &tauri::AppHandle) -> Result<(), String> {
    app.state::<CompanionRelay>()
        .0
        .lock()
        .map_err(|_| "Could not stop companion")?
        .take();
    if let Some(window) = app.get_webview_window("companion") {
        window
            .clear_all_browsing_data()
            .map_err(|_| "Could not clear companion credentials")?;
        window.destroy().map_err(|_| "Could not close companion")?;
    }
    Ok(())
}

/// Clear the old persistent webview profile, including remote-origin localStorage,
/// all host/domain/path variants of bolly_token, caches and service workers.
/// Plugin settings are outside the webview profile and are migrated separately.
#[tauri::command]
async fn clear_legacy_browser_auth(app: tauri::AppHandle) -> Result<(), String> {
    let browser = if let Some(window) = app.get_webview_window("main") {
        window
            .clear_all_browsing_data()
            .map_err(|_| "Could not clear legacy browser credentials".to_string())
    } else {
        Ok(())
    };
    // plugin-http used to enable its persistent cookie jar by default. It is
    // disabled now; also delete the old jar so it cannot retain credentials.
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|_| "Could not locate legacy HTTP credentials")?;
    let http = match std::fs::remove_file(cache.join(".cookies")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Could not clear legacy HTTP credentials".to_string()),
    };
    browser.and(http)
}

fn connection_url(input: &str) -> Result<url::Url, String> {
    let parsed = url::Url::parse(input).map_err(|_| "Invalid server URL")?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
    {
        return Err("Invalid server URL".into());
    }
    Ok(parsed)
}

// Serialize lifecycle changes, including validation, so an older open cannot
// finish after a newer disconnect.
static CONNECTION_LIFECYCLE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn validate_connection(url: &str, token: &str) -> Result<(), String> {
    let mut target = connection_url(url)?;
    if token.trim().is_empty() || token.contains(['\r', '\n']) {
        return Err("Invalid auth token".into());
    }
    target.set_path("/api/meta");
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| "Could not validate connection")?
        .get(target)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| "Could not reach server")?;
    if !response.status().is_success() {
        return Err("Server rejected connection".into());
    }
    let meta: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Invalid server metadata")?;
    if meta["app"] != "bolly" {
        return Err("Invalid server metadata".into());
    }
    Ok(())
}

#[tauri::command]
async fn test_connection(url: String, token: String) -> Result<(), String> {
    validate_connection(&url, &token).await
}

#[tauri::command]
async fn initialize_saved_connection(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_store::StoreExt;
    // Retire the old JS-writable credential metadata. It cannot be safely bound
    // after the fact, so remove its keychain entry rather than ever using its URL.
    let store = app
        .store("settings.json")
        .map_err(|_| "Could not load legacy settings")?;
    let mut legacy_refs = Vec::new();
    if let Some(reference) = store
        .get("connection")
        .and_then(|value| value["secretRef"].as_str().map(str::to_owned))
    {
        legacy_refs.push(reference);
    }
    if let Some(values) = store
        .get("secret_cleanup")
        .and_then(|value| value.as_array().cloned())
    {
        legacy_refs.extend(
            values
                .into_iter()
                .filter_map(|value| value.as_str().map(str::to_owned)),
        );
    }
    for key in ["session", "self_hosted", "connection", "secret_cleanup"] {
        store.delete(key);
    }
    store
        .save()
        .map_err(|_| "Could not clear legacy settings")?;
    for reference in legacy_refs {
        credentials::delete_legacy(reference).await?;
    }
    Ok(credentials::read()
        .await?
        .map(|connection| connection.origin))
}

#[tauri::command]
async fn save_connection(
    app: tauri::AppHandle,
    url: String,
    token: String,
) -> Result<String, String> {
    let _guard = CONNECTION_LIFECYCLE.lock().await;
    let origin = connection_url(&url)?.origin().ascii_serialization();
    let token = if token.is_empty() {
        let saved = credentials::read().await?.ok_or("No saved connection")?;
        if saved.origin != origin {
            return Err("Changing the server URL requires its auth token".into());
        }
        saved.token
    } else {
        token
    };
    validate_connection(&origin, &token).await?;
    computer_use_bridge::disconnect_computer_use(app.clone()).await?;
    close_companion(&app)?;
    credentials::store(credentials::SavedConnection {
        origin: origin.clone(),
        token,
    })
    .await?;
    Ok(origin)
}

#[tauri::command]
async fn test_saved_connection() -> Result<(), String> {
    let saved = credentials::read().await?.ok_or("No saved connection")?;
    validate_connection(&saved.origin, &saved.token).await
}

#[tauri::command]
async fn open_saved_connection(app: tauri::AppHandle) -> Result<(), String> {
    let _guard = CONNECTION_LIFECYCLE.lock().await;
    let saved = credentials::read().await?.ok_or("No saved connection")?;
    validate_connection(&saved.origin, &saved.token).await?;
    computer_use_bridge::connect_computer_use(
        app.clone(),
        saved.origin.clone(),
        saved.token.clone(),
    )
    .await?;
    if let Err(error) = navigate(app.clone(), saved.origin, saved.token).await {
        computer_use_bridge::disconnect_computer_use(app).await?;
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
async fn delete_saved_connection() -> Result<(), String> {
    credentials::delete().await
}

#[tauri::command]
async fn disconnect_computer_use(app: tauri::AppHandle) -> Result<(), String> {
    let _guard = CONNECTION_LIFECYCLE.lock().await;
    // Attempt all cleanup even when one operation fails; the UI can retry.
    let bridge = computer_use_bridge::disconnect_computer_use(app.clone()).await;
    let companion = close_companion(&app);
    let legacy = clear_legacy_browser_auth(app.clone()).await;
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
    }
    bridge.and(companion).and(legacy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn metadata_validation_is_native_authenticated_and_never_follows_redirects() {
        use axum::{
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
            routing::get,
            Router,
        };
        for (status, body) in [
            (200, r#"{"app":"bolly"}"#),
            (200, r#"{"app":"other"}"#),
            (401, "TOP_SECRET"),
            (302, "TOP_SECRET"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let app = Router::new().route(
                "/api/meta",
                get(move |headers: HeaderMap| async move {
                    assert_eq!(headers["authorization"], "Bearer TOP_SECRET");
                    (
                        StatusCode::from_u16(status).unwrap(),
                        [("location", "http://127.0.0.1:1/secret")],
                        body,
                    )
                        .into_response()
                }),
            );
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let result = test_connection(origin, "TOP_SECRET".into()).await;
            assert_eq!(result.is_ok(), status == 200 && body.contains("bolly"));
            assert!(!format!("{result:?}").contains("TOP_SECRET"));
            server.abort();
        }
    }

    #[test]
    fn server_urls_reject_paths_and_preserve_ports() {
        assert!(connection_url("https://example.org:8443/bolly").is_err());
        let url = connection_url("https://example.org:8443/").unwrap();
        assert_eq!(url.path(), "/");
        assert_eq!(
            url.origin().ascii_serialization(),
            "https://example.org:8443"
        );
        assert_ne!(
            url.origin(),
            connection_url("https://example.org").unwrap().origin()
        );
    }

    #[test]
    fn unrelated_localhost_ports_are_not_internal_app_urls() {
        assert!(!is_internal_url(
            &connection_url("http://localhost:3000").unwrap()
        ));
        assert!(is_internal_url(
            &url::Url::parse("tauri://localhost/settings").unwrap()
        ));
    }

    #[test]
    fn companion_navigation_is_confined_to_its_exact_relay_origin() {
        let origin = "http://127.0.0.1:43123";
        assert!(companion_allows_navigation(
            &url::Url::parse("http://127.0.0.1:43123/chat").unwrap(),
            origin
        ));
        for target in [
            "https://example.org/",
            "http://127.0.0.1:43124/",
            "javascript:alert(1)",
            "data:text/html,hello",
        ] {
            assert!(!companion_allows_navigation(
                &url::Url::parse(target).unwrap(),
                origin
            ));
        }
    }

    #[test]
    fn reject_credentials_and_non_server_urls() {
        for input in [
            "file:///tmp/a",
            "javascript:alert(1)",
            "https://user:secret@example.org",
            "https://example.org/?token=secret",
            "https://example.org/#secret",
        ] {
            assert!(connection_url(input).is_err());
        }
    }
}

fn navigate_home(app: &tauri::AppHandle) -> Result<(), String> {
    if let Some(companion) = app.get_webview_window("companion") {
        companion.hide().map_err(|_| "Could not hide companion")?;
    }
    let main = app
        .get_webview_window("main")
        .ok_or("main webview not found")?;
    main.show().map_err(|_| "Could not show dashboard")?;
    main.set_focus()
        .map_err(|_| "Could not focus dashboard".into())
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
        .plugin(tauri_plugin_store::Builder::default().build());

    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }

    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            test_connection,
            initialize_saved_connection,
            save_connection,
            test_saved_connection,
            open_saved_connection,
            delete_saved_connection,
            clear_legacy_browser_auth,
            computer_use::computer_screenshot,
            computer_use::computer_click,
            computer_use::computer_double_click,
            computer_use::computer_mouse_move,
            computer_use::computer_scroll,
            computer_use::computer_type,
            computer_use::computer_key,
            disconnect_computer_use,
            computer_use_bridge::set_screen_recording_allowed,
            computer_use_bridge::get_screen_recording_allowed,
            computer_use_bridge::get_server_url,
            computer_use_bridge::stop_screen_recording,
            computer_use_bridge::set_instance_slug,
            permissions::check_permissions,
            permissions::open_permission_settings,
        ])
        .setup(|app| {
            app.manage(CompanionRelay(Mutex::new(None)));

            // Single instance must be registered first in setup
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                app.handle().plugin(tauri_plugin_single_instance::init(
                    move |_app, _argv, _cwd| {
                        // Focus existing window
                        if let Some(win) = handle.get_webview_window("main") {
                            win.set_focus().ok();
                        }
                    },
                ))?;
            }
            let about = AboutMetadataBuilder::new()
                .name(Some("Bolly"))
                .version(Some(env!("CARGO_PKG_VERSION")))
                .website(Some("https://github.com/triangle-int/bolly"))
                .website_label(Some("Bolly on GitHub"))
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
                    let _ = nav_handle.opener().open_url(url.as_str(), None::<&str>);
                    false
                })
                .build()?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
