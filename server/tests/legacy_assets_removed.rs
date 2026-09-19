//! Guard for #99: release artifacts contain only reachable Nolune / Little
//! Moon code and assets, no retired amber-orb media, no placeholder packaging,
//! and every retained static asset has a live reference.

use std::{
    fs,
    path::{Path, PathBuf},
};

fn files(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    fn visit(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, extensions, out);
            } else if extensions.is_empty()
                || path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| extensions.contains(&ext))
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    visit(root, extensions, &mut out);
    out.sort();
    out
}

fn sources(repo: &Path, dir: &str) -> String {
    files(
        &repo.join(dir),
        &["svelte", "ts", "js", "html", "css", "json"],
    )
    .into_iter()
    .map(|path| fs::read_to_string(path).unwrap_or_default())
    .collect::<Vec<_>>()
    .join("\n")
}

#[test]
fn unreachable_legacy_clusters_media_and_placeholder_packaging_are_gone() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut violations = Vec::new();

    for removed in [
        // client: 3D/ASCII creature cluster and unused primitives
        "client/src/lib/components/chat/ActivityEvent.svelte",
        "client/src/lib/components/chat/AsciiCreature.svelte",
        "client/src/lib/components/chat/AsciiRenderer.svelte",
        "client/src/lib/components/chat/AsciiShader.svelte",
        "client/src/lib/components/chat/CreatureScene.svelte",
        "client/src/lib/components/chat/MindCard.svelte",
        "client/src/lib/components/chat/PresenceHeader.svelte",
        "client/src/lib/components/soul/SoulEditor.svelte",
        "client/static/icons",
        // landing: glass-sphere scenes, transition, pricing art, marketplace, demo
        "landing/src/lib/components/ComputerUse.svelte",
        "landing/src/lib/components/Demo.svelte",
        "landing/src/lib/components/Features.svelte",
        "landing/src/lib/components/GlassScene.svelte",
        "landing/src/lib/components/GlassSceneContent.svelte",
        "landing/src/lib/components/GlassSpheres.svelte",
        "landing/src/lib/components/GlassSpheresScene.svelte",
        "landing/src/lib/components/HowItWorks.svelte",
        "landing/src/lib/components/LiquidGlassSpheres.svelte",
        "landing/src/lib/components/Pricing.svelte",
        "landing/src/lib/components/ScrollReveal.svelte",
        "landing/src/lib/components/SphereOverlay.svelte",
        "landing/src/lib/components/SphereOverlayContent.svelte",
        "landing/src/lib/components/Transition.svelte",
        "landing/static/assets/hero-orb.mp4",
        "landing/static/assets/hero-orb-connected.mp4",
        "landing/static/assets/hero-orb.webp",
        "landing/static/assets/marketplace-cover.png",
        "landing/static/assets/marketplace-logo.png",
        "landing/static/assets/plan-companion.png",
        "landing/static/assets/plan-friend.png",
        "landing/static/assets/plan-starter.png",
        "landing/static/assets/transition-shatter.mp4",
        "landing/static/assets/demo.mp4",
        // packaging template and analysis scripts
        "desktop/flatpak",
        "scripts/cache-ttl-cost.py",
        "scripts/cost-to-1m.py",
        "scripts/daily-cost.py",
    ] {
        if repo.join(removed).exists() {
            violations.push(format!("still exists: {removed}"));
        }
    }
    for feature in files(&repo.join("landing/static/assets"), &[]) {
        let name = feature.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("feature-") {
            violations.push(format!("retired feature media still exists: {name}"));
        }
    }

    for required in [
        "landing/src/lib/components/Install.svelte",
        "landing/static/assets/favicon.svg",
        "landing/static/apple-touch-icon.png",
        "client/src/lib/components/SharedScene.svelte",
    ] {
        if !repo.join(required).exists() {
            violations.push(format!("missing: {required}"));
        }
    }

    for package in ["client/package.json", "landing/package.json"] {
        let manifest = fs::read_to_string(repo.join(package)).unwrap();
        for dep in ["\"three\"", "\"@threlte/core\"", "\"@threlte/extras\""] {
            if manifest.contains(dep) {
                violations.push(format!("{package} still depends on {dep}"));
            }
        }
    }
    for (file, forbidden) in [
        ("desktop/README.md", "Flatpak"),
        ("docs/nolune-cutover.md", "Flatpak"),
        ("desktop/src-tauri/tauri.conf.json", "PLACEHOLDER"),
    ] {
        let text = fs::read_to_string(repo.join(file)).unwrap();
        if text.contains(forbidden) {
            violations.push(format!("{file} still mentions {forbidden:?}"));
        }
    }

    assert!(
        violations.is_empty(),
        "legacy surface remains:\n{}",
        violations.join("\n")
    );
}

#[test]
fn every_retained_static_asset_has_a_live_reference() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut violations = Vec::new();

    let landing_src =
        sources(repo, "landing/src") + &fs::read_to_string(repo.join("README.md")).unwrap();
    for asset in files(&repo.join("landing/static/assets"), &[]) {
        let name = asset.file_name().unwrap().to_string_lossy().into_owned();
        if !landing_src.contains(&name) {
            violations.push(format!("landing/static/assets/{name} is not referenced"));
        }
    }
    // Root-level files browsers fetch by convention.
    for conventional in ["apple-touch-icon.png", "robots.txt"] {
        if !repo.join("landing/static").join(conventional).exists() {
            violations.push(format!("landing/static/{conventional} is missing"));
        }
    }

    let client_src =
        sources(repo, "client/src") + &fs::read_to_string(repo.join("README.md")).unwrap();
    for sound in files(&repo.join("client/static/sounds"), &["mp3"]) {
        let stem = sound.file_stem().unwrap().to_string_lossy().into_owned();
        if !client_src.contains(&format!("\"{stem}\""))
            && !client_src.contains(&format!("'{stem}'"))
        {
            violations.push(format!("client/static/sounds/{stem}.mp3 is never played"));
        }
    }
    for skin in files(&repo.join("client/static/skins"), &[]) {
        let name = skin.file_name().unwrap().to_string_lossy().into_owned();
        if !client_src.contains(&name) {
            violations.push(format!(
                "client/static/skins asset {name} is not referenced"
            ));
        }
    }
    for extra in files(&repo.join("client/static"), &[]) {
        let relative = extra.strip_prefix(repo.join("client/static")).unwrap();
        let top = relative
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .into_owned();
        if !["sounds", "skins", "robots.txt"].contains(&top.as_str()) {
            violations.push(format!(
                "unexpected client static asset: {}",
                relative.display()
            ));
        }
    }

    let desktop_src = sources(repo, "desktop/src");
    for asset in files(&repo.join("desktop/static"), &[]) {
        let name = asset.file_name().unwrap().to_string_lossy().into_owned();
        if !desktop_src.contains(&name) {
            violations.push(format!("desktop/static/{name} is not referenced"));
        }
    }

    assert!(
        violations.is_empty(),
        "unreferenced or missing assets:\n{}",
        violations.join("\n")
    );
}
