use base64::{engine::general_purpose::STANDARD, Engine};
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
use image::DynamicImage;
use serde::Serialize;
use std::io::Cursor;
use std::thread;
use std::time::Duration;

/// Max pixels on the longest edge for screenshots sent to Claude.
/// Anthropic recommends 1568 but we use 1280 to keep file size under 1MB.
const MAX_SCREENSHOT_EDGE: u32 = 1280;

// ─── Screenshot ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ScreenshotResult {
    pub image: String,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

#[tauri::command]
pub fn computer_screenshot() -> Result<ScreenshotResult, String> {
    let screens = screenshots::Screen::all().map_err(|e| e.to_string())?;
    let screen = screens.into_iter().next().ok_or("no screen found")?;

    // Display scale factor (2.0 on Retina, 1.0 on standard displays)
    let display_scale = screen.display_info.scale_factor as f64;
    // Logical screen dimensions (what enigo uses for mouse coordinates)
    let logical_w = screen.display_info.width as f64;
    let _logical_h = screen.display_info.height as f64;

    let capture = screen.capture().map_err(|e| e.to_string())?;
    let real_w = capture.width();
    let real_h = capture.height();
    let raw = capture.into_raw();

    let rgba =
        image::RgbaImage::from_raw(real_w, real_h, raw).ok_or("failed to create image buffer")?;
    let img = DynamicImage::ImageRgba8(rgba);

    // Scale so the longest edge ≤ MAX_SCREENSHOT_EDGE
    let longest = real_w.max(real_h);
    let (scaled_img, scale) = if longest > MAX_SCREENSHOT_EDGE {
        let ratio = MAX_SCREENSHOT_EDGE as f64 / longest as f64;
        let new_w = (real_w as f64 * ratio).round() as u32;
        let new_h = (real_h as f64 * ratio).round() as u32;
        let resized = img.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle);
        // Scale from screenshot coords → logical screen coords (not physical pixels!)
        // Screenshot is scaled down from physical pixels, but enigo uses logical coords.
        // So: screenshot_coord * scale = logical_coord
        let scale = logical_w / new_w as f64;
        (resized, scale)
    } else {
        // No resize needed — scale from physical pixels to logical
        (img, 1.0 / display_scale)
    };

    let out_w = scaled_img.width();
    let out_h = scaled_img.height();

    // Encode as JPEG (much smaller than PNG — ~200KB vs 4MB)
    let rgb_img = scaled_img.to_rgb8();
    let mut buf = Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 75)
        .encode(
            rgb_img.as_raw(),
            out_w,
            out_h,
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| e.to_string())?;

    let b64 = STANDARD.encode(buf.into_inner());

    Ok(ScreenshotResult {
        image: b64,
        width: out_w,
        height: out_h,
        scale,
    })
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn scale_coords(x: i32, y: i32, scale: f64) -> (i32, i32) {
    (
        (x as f64 * scale).round() as i32,
        (y as f64 * scale).round() as i32,
    )
}

fn new_enigo() -> Result<Enigo, String> {
    Enigo::new(&Settings::default()).map_err(|e| e.to_string())
}

// ─── Mouse ───────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn computer_click(x: i32, y: i32, scale: f64, button: String) -> Result<(), String> {
    let (rx, ry) = scale_coords(x, y, scale);
    let mut enigo = new_enigo()?;
    enigo
        .move_mouse(rx, ry, Coordinate::Abs)
        .map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(50));

    let btn = match button.as_str() {
        "right" => Button::Right,
        "middle" => Button::Middle,
        _ => Button::Left,
    };
    enigo
        .button(btn, Direction::Click)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn computer_double_click(x: i32, y: i32, scale: f64) -> Result<(), String> {
    let (rx, ry) = scale_coords(x, y, scale);
    let mut enigo = new_enigo()?;
    enigo
        .move_mouse(rx, ry, Coordinate::Abs)
        .map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(50));
    enigo
        .button(Button::Left, Direction::Click)
        .map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(80));
    enigo
        .button(Button::Left, Direction::Click)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn computer_mouse_move(x: i32, y: i32, scale: f64) -> Result<(), String> {
    let (rx, ry) = scale_coords(x, y, scale);
    let mut enigo = new_enigo()?;
    enigo
        .move_mouse(rx, ry, Coordinate::Abs)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn computer_scroll(
    x: i32,
    y: i32,
    scale: f64,
    delta_x: i32,
    delta_y: i32,
) -> Result<(), String> {
    let (rx, ry) = scale_coords(x, y, scale);
    let mut enigo = new_enigo()?;
    enigo
        .move_mouse(rx, ry, Coordinate::Abs)
        .map_err(|e| e.to_string())?;
    thread::sleep(Duration::from_millis(50));

    if delta_y != 0 {
        enigo
            .scroll(delta_y, Axis::Vertical)
            .map_err(|e| e.to_string())?;
    }
    if delta_x != 0 {
        enigo
            .scroll(delta_x, Axis::Horizontal)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ─── Keyboard ────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn computer_type(text: String) -> Result<(), String> {
    let mut enigo = new_enigo()?;
    enigo.text(&text).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn computer_key(key: String) -> Result<(), String> {
    use enigo::Key;

    let mut enigo = new_enigo()?;

    let parts: Vec<&str> = key.split('+').map(|s| s.trim()).collect();
    let mut modifiers: Vec<Key> = Vec::new();
    let mut main_key: Option<Key> = None;

    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;
        let parsed = parse_key(part);

        if is_last {
            main_key = Some(parsed);
        } else {
            modifiers.push(parsed);
        }
    }

    for m in &modifiers {
        enigo.key(*m, Direction::Press).map_err(|e| e.to_string())?;
    }

    if let Some(k) = main_key {
        enigo.key(k, Direction::Click).map_err(|e| e.to_string())?;
    }

    for m in modifiers.iter().rev() {
        enigo
            .key(*m, Direction::Release)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

fn parse_key(name: &str) -> enigo::Key {
    use enigo::Key;

    match name.to_lowercase().as_str() {
        "ctrl" | "control" => Key::Control,
        "alt" | "option" => Key::Alt,
        "shift" => Key::Shift,
        "cmd" | "command" | "meta" | "super" => Key::Meta,
        "return" | "enter" => Key::Return,
        "tab" => Key::Tab,
        "escape" | "esc" => Key::Escape,
        "space" => Key::Space,
        "backspace" | "delete" => Key::Backspace,
        "forwarddelete" | "del" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "page_up" => Key::PageUp,
        "pagedown" | "page_down" => Key::PageDown,
        "up" | "arrowup" => Key::UpArrow,
        "down" | "arrowdown" => Key::DownArrow,
        "left" | "arrowleft" => Key::LeftArrow,
        "right" | "arrowright" => Key::RightArrow,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        other => {
            if other.len() == 1 {
                Key::Unicode(other.chars().next().unwrap())
            } else {
                Key::Unicode(' ')
            }
        }
    }
}
