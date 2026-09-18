use tauri::{AppHandle, Emitter, Manager};

#[cfg(target_os = "macos")]
use tauri::WebviewUrl;
#[cfg(target_os = "macos")]
use tauri_nspanel::{
    tauri_panel, CollectionBehavior, ManagerExt, PanelBuilder, PanelLevel, StyleMask,
};

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(OverlayPanel {
        config: {
            can_become_key_window: false,
            can_become_main_window: false,
            is_floating_panel: true,
            hides_on_deactivate: false,
        }
    })
}

/// Overlay URL — always local Tauri page. Video loads from server via src attribute.
fn overlay_url_str(_slug: Option<&str>) -> String {
    if cfg!(debug_assertions) {
        "http://localhost:1420/overlay/".to_string()
    } else {
        "tauri://localhost/overlay/".to_string()
    }
}

/// Show the overlay — NSPanel on macOS, regular window on other platforms.
pub fn show(app: &AppHandle) {
    show_with_slug(app, None);
}

/// Show the overlay for a specific instance slug.
pub fn show_for_instance(app: &AppHandle, slug: &str) {
    show_with_slug(app, Some(slug.to_string()));
}

fn show_with_slug(app: &AppHandle, slug: Option<String>) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        {
            show_macos(&inner, slug.as_deref());
        }
        #[cfg(not(target_os = "macos"))]
        {
            show_fallback(&inner, slug.as_deref());
        }
    });
}

#[cfg(target_os = "macos")]
fn show_macos(app: &AppHandle, slug: Option<&str>) {
    if let Ok(panel) = app.get_webview_panel("overlay") {
        if !panel.is_visible() {
            panel.show();
        }
        return;
    }

    let url: WebviewUrl = WebviewUrl::External(overlay_url_str(slug).parse().unwrap());

    match PanelBuilder::<_, OverlayPanel>::new(app, "overlay")
        .url(url)
        .transparent(true)
        .opaque(false)
        .has_shadow(false)
        .level(PanelLevel::MainMenu)
        .collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .full_screen_auxiliary()
                .stationary()
                .ignores_cycle(),
        )
        .ignores_mouse_events(true)
        .style_mask(StyleMask::empty().borderless().nonactivating_panel())
        .with_window(|w| {
            w.decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .transparent(true)
                .maximized(true)
        })
        .no_activate(true)
        .build()
    {
        Ok(panel) => {
            panel.show();
            eprintln!("[overlay] NSPanel shown (all spaces + fullscreen)");
        }
        Err(e) => eprintln!("[overlay] panel build error: {e}"),
    }
}

#[cfg(not(target_os = "macos"))]
fn show_fallback(app: &AppHandle, slug: Option<&str>) {
    if app.get_webview_window("overlay").is_some() {
        return;
    }

    let parsed: url::Url = match overlay_url_str(slug).parse() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("[overlay] bad URL: {e}");
            return;
        }
    };

    match tauri::webview::WebviewWindowBuilder::new(
        app,
        "overlay",
        tauri::WebviewUrl::External(parsed),
    )
    .title("")
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .visible_on_all_workspaces(true)
    .skip_taskbar(true)
    .focused(false)
    .resizable(false)
    .maximized(true)
    .build()
    {
        Ok(win) => {
            let _ = win.set_ignore_cursor_events(true);
            eprintln!("[overlay] fallback window shown");
        }
        Err(e) => eprintln!("[overlay] build error: {e}"),
    }
}

/// Hide the overlay (don't destroy — reuse on next show).
pub fn hide(app: &AppHandle) {
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        {
            if let Ok(panel) = inner.get_webview_panel("overlay") {
                panel.hide();
                eprintln!("[overlay] hidden (panel)");
                return;
            }
        }
        if let Some(win) = inner.get_webview_window("overlay") {
            let _ = win.hide();
            eprintln!("[overlay] hidden (window)");
        }
    });
}

/// Notify the overlay of a computer-use action with full detail.
pub fn emit_action_detail(app: &AppHandle, action: &str, detail: &str) {
    let payload = serde_json::json!({ "action": action, "detail": detail });
    app.emit("computer-use-action", payload.to_string()).ok();
}

/// Signal idle.
pub fn emit_idle(app: &AppHandle) {
    app.emit("computer-use-idle", ()).ok();
}

/// Temporarily hide/show the overlay without destroying it.
pub fn set_visible(app: &AppHandle, visible: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        #[cfg(target_os = "macos")]
        {
            if let Ok(panel) = handle.get_webview_panel("overlay") {
                if visible {
                    panel.show();
                } else {
                    panel.hide();
                }
                return;
            }
        }
        if let Some(win) = handle.get_webview_window("overlay") {
            if visible {
                let _ = win.show();
            } else {
                let _ = win.hide();
            }
        }
    });
}
