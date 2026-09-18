//! Screen capture via the `screenshots` crate (cross-platform).
//! Takes a screenshot every second and stores it as JPEG in memory.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::Duration;

static STREAMING: AtomicBool = AtomicBool::new(false);
static LAST_FRAME: Mutex<Option<Vec<u8>>> = Mutex::new(None);

pub fn start() -> Result<(), String> {
    if STREAMING.load(Ordering::Relaxed) {
        return Ok(());
    }

    // Quick check that we can capture
    screenshots::Screen::all().map_err(|e| format!("no screens: {e}"))?;

    STREAMING.store(true, Ordering::Relaxed);
    std::thread::spawn(|| {
        if let Err(e) = capture_loop() {
            eprintln!("[recorder] error: {e}");
        }
        STREAMING.store(false, Ordering::Relaxed);
    });
    eprintln!("[recorder] started (screenshots, 1fps)");
    Ok(())
}

pub fn stop() -> Result<(), String> {
    STREAMING.store(false, Ordering::Relaxed);
    eprintln!("[recorder] stopped");
    Ok(())
}

pub fn get_last_frame() -> Option<Vec<u8>> {
    LAST_FRAME.lock().ok()?.clone()
}

pub fn is_recording() -> bool {
    STREAMING.load(Ordering::Relaxed)
}

fn capture_loop() -> Result<(), String> {
    while STREAMING.load(Ordering::Relaxed) {
        match take_screenshot_jpeg() {
            Ok(jpeg) => {
                if let Ok(mut g) = LAST_FRAME.lock() {
                    *g = Some(jpeg);
                }
            }
            Err(e) => eprintln!("[recorder] screenshot error: {e}"),
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

fn take_screenshot_jpeg() -> Result<Vec<u8>, String> {
    use image::DynamicImage;
    use std::io::Cursor;

    let screens = screenshots::Screen::all().map_err(|e| e.to_string())?;
    let screen = screens.into_iter().next().ok_or("no screen")?;
    let capture = screen.capture().map_err(|e| e.to_string())?;

    let w = capture.width();
    let h = capture.height();
    let rgba = image::RgbaImage::from_raw(w, h, capture.into_raw()).ok_or("bad image buffer")?;
    let img = DynamicImage::ImageRgba8(rgba);

    // Scale down for efficiency
    let img = if w > 1280 {
        let ratio = 1280.0 / w as f64;
        let new_h = (h as f64 * ratio) as u32;
        img.resize_exact(1280, new_h, image::imageops::FilterType::Triangle)
    } else {
        img
    };

    let rgb = img.to_rgb8();
    let mut buf = Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 50)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| e.to_string())?;

    Ok(buf.into_inner())
}
