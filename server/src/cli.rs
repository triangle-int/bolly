use std::process::Command;

use clap::{Parser, Subcommand};

use crate::config;

const PLIST_LABEL: &str = "dev.nolune.nolune";

#[derive(Parser)]
#[command(
    name = "nolune",
    about = "Nolune — AI companion",
    version = env!("CARGO_PKG_VERSION"),
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

#[derive(Subcommand)]
pub enum CliCommand {
    /// Start the Nolune background service
    Start,
    /// Stop the Nolune background service
    Stop,
    /// Restart the Nolune background service
    Restart,
    /// Show service status
    Status,
    /// Stream logs in real time
    Logs,
    /// Print version
    Version,
    /// Create a one-time code so a browser can sign in to this server
    Pair,
}

pub fn run(cmd: CliCommand) -> i32 {
    match cmd {
        CliCommand::Start => svc_start(),
        CliCommand::Stop => svc_stop(),
        CliCommand::Restart => {
            svc_stop();
            svc_start()
        }
        CliCommand::Status => svc_status(),
        CliCommand::Logs => svc_logs(),
        CliCommand::Version => {
            println!("nolune {}", env!("CARGO_PKG_VERSION"));
            0
        }
        CliCommand::Pair => pair(),
    }
}

// ── Browser pairing ─────────────────────────────────────────────────────

/// Ask the running server for a pairing code and print it. This is the local
/// owner surface for #112: the code is short-lived and single-use, and the
/// API token itself never leaves this process.
fn pair() -> i32 {
    let config = match config::load_config() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("cannot read {}: {error}", config::config_path().display());
            return 1;
        }
    };
    if config.auth_token.is_empty() {
        println!(
            "Authentication is disabled (auth_token is empty in {}), so browsers can open Nolune without pairing.",
            config::config_path().display()
        );
        return 0;
    }

    let connect_host = match config.host.as_str() {
        "" | "0.0.0.0" => "127.0.0.1".to_string(),
        "::" | "[::]" => "[::1]".to_string(),
        other => other.to_string(),
    };
    let url = format!("http://{connect_host}:{}/api/session/pairing", config.port);
    let open_url = if config.public_url.is_empty() {
        format!("http://localhost:{}", config.port)
    } else {
        config.public_url.clone()
    };

    // `main` is already inside the tokio runtime, so drive this request on a
    // fresh thread with its own runtime.
    let auth_token = config.auth_token.clone();
    let request_url = url.clone();
    let response = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        runtime.block_on(async {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .map_err(|e| e.to_string())?;
            let response = client
                .post(&request_url)
                .bearer_auth(&auth_token)
                .json(&serde_json::json!({ "source": "cli" }))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let status = response.status();
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            Ok::<_, String>((status, body))
        })
    })
    .join()
    .unwrap_or_else(|_| Err("pairing thread panicked".to_string()));

    match response {
        Err(error) => {
            eprintln!(
                "Nolune is not reachable at {url} ({error}).
Start it with `nolune start` and try again."
            );
            1
        }
        Ok((status, _)) if status == reqwest::StatusCode::UNAUTHORIZED => {
            eprintln!(
                "The running server rejected the token from {}.
If the service was started with NOLUNE_AUTH_TOKEN, run `nolune pair` with the same value.",
                config::config_path().display()
            );
            1
        }
        Ok((status, body)) if !status.is_success() => {
            let message = body["message"].as_str().unwrap_or("");
            eprintln!("pairing request failed: HTTP {status} {message}");
            1
        }
        Ok((_, body)) => {
            let Some(code) = body["code"].as_str() else {
                eprintln!("the server did not return a pairing code");
                return 1;
            };
            let minutes = body["expires_in_secs"].as_u64().unwrap_or(300) / 60;
            println!();
            println!("  Pairing code:  {code}");
            println!();
            println!("  Open {open_url} in the browser you want to connect and enter this code.");
            println!(
                "  It works once and expires in {minutes} minutes. Run `nolune pair` again for another browser."
            );
            println!();
            0
        }
    }
}

// ── macOS (launchd) ─────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn plist_path() -> String {
    let home = dirs::home_dir().expect("cannot resolve home directory");
    format!(
        "{}/Library/LaunchAgents/{PLIST_LABEL}.plist",
        home.display()
    )
}

#[cfg(target_os = "macos")]
fn uid() -> String {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .expect("failed to run `id`");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[cfg(target_os = "macos")]
fn svc_start() -> i32 {
    let plist = plist_path();
    if !std::path::Path::new(&plist).exists() {
        eprintln!("plist not found at {plist} — was Nolune installed with the install script?");
        return 1;
    }
    let domain = format!("gui/{}", uid());
    let status = Command::new("launchctl")
        .args(["bootstrap", &domain, &plist])
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("Nolune service started.");
            0
        }
        Ok(s) => {
            // exit code 37 = already loaded
            if s.code() == Some(37) {
                println!("Nolune service is already running.");
                0
            } else {
                eprintln!(
                    "launchctl bootstrap failed (exit {}).",
                    s.code().unwrap_or(-1)
                );
                1
            }
        }
        Err(e) => {
            eprintln!("failed to run launchctl: {e}");
            1
        }
    }
}

#[cfg(target_os = "macos")]
fn svc_stop() -> i32 {
    let target = format!("gui/{}/{PLIST_LABEL}", uid());
    let status = Command::new("launchctl")
        .args(["bootout", &target])
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("Nolune service stopped.");
            0
        }
        Ok(s) => {
            if s.code() == Some(3) {
                println!("Nolune service is not running.");
                0
            } else {
                eprintln!(
                    "launchctl bootout failed (exit {}).",
                    s.code().unwrap_or(-1)
                );
                1
            }
        }
        Err(e) => {
            eprintln!("failed to run launchctl: {e}");
            1
        }
    }
}

#[cfg(target_os = "macos")]
fn svc_status() -> i32 {
    let target = format!("gui/{}/{PLIST_LABEL}", uid());
    let output = Command::new("launchctl").args(["print", &target]).output();
    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            if out.status.success() {
                let state = text
                    .lines()
                    .find(|l| l.trim().starts_with("state ="))
                    .map(|l| l.trim().trim_start_matches("state = "))
                    .unwrap_or("unknown");
                let pid = text
                    .lines()
                    .find(|l| l.trim().starts_with("pid ="))
                    .map(|l| l.trim().trim_start_matches("pid = "))
                    .unwrap_or("-");
                println!("Nolune is running (pid {pid}, state: {state})");
            } else {
                println!("Nolune is not running.");
            }
            0
        }
        Err(e) => {
            eprintln!("failed to run launchctl: {e}");
            1
        }
    }
}

#[cfg(target_os = "macos")]
fn svc_logs() -> i32 {
    let log_path = config::workspace_root().join("nolune.log");
    if !log_path.exists() {
        eprintln!("log file not found at {}", log_path.display());
        return 1;
    }
    let status = Command::new("tail")
        .args(["-f", &log_path.to_string_lossy()])
        .status();
    match status {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("failed to tail logs: {e}");
            1
        }
    }
}

// ── Linux (systemd) ─────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn svc_start() -> i32 {
    run_systemctl(&["start", "nolune"], "started")
}

#[cfg(target_os = "linux")]
fn svc_stop() -> i32 {
    run_systemctl(&["stop", "nolune"], "stopped")
}

#[cfg(target_os = "linux")]
fn svc_status() -> i32 {
    let output = Command::new("systemctl")
        .args(["is-active", "nolune"])
        .output();
    match output {
        Ok(out) => {
            let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if state == "active" {
                println!("Nolune is running.");
            } else {
                println!("Nolune is not running ({state}).");
            }
            0
        }
        Err(e) => {
            eprintln!("failed to run systemctl: {e}");
            1
        }
    }
}

#[cfg(target_os = "linux")]
fn svc_logs() -> i32 {
    let status = Command::new("journalctl")
        .args(["-u", "nolune", "-f", "--no-pager"])
        .status();
    match status {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("failed to run journalctl: {e}");
            1
        }
    }
}

#[cfg(target_os = "linux")]
fn run_systemctl(args: &[&str], verb: &str) -> i32 {
    // Try user-level first, fall back to sudo system-level
    let user = Command::new("systemctl").arg("--user").args(args).status();
    if let Ok(s) = user {
        if s.success() {
            println!("Nolune service {verb}.");
            return 0;
        }
    }
    let system = Command::new("sudo").arg("systemctl").args(args).status();
    match system {
        Ok(s) if s.success() => {
            println!("Nolune service {verb}.");
            0
        }
        Ok(s) => {
            eprintln!("systemctl failed (exit {}).", s.code().unwrap_or(-1));
            1
        }
        Err(e) => {
            eprintln!("failed to run systemctl: {e}");
            1
        }
    }
}

// ── Unsupported platform ────────────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn svc_start() -> i32 {
    eprintln!("service management is not supported on this platform");
    1
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn svc_stop() -> i32 {
    eprintln!("service management is not supported on this platform");
    1
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn svc_status() -> i32 {
    eprintln!("service management is not supported on this platform");
    1
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn svc_logs() -> i32 {
    eprintln!("service management is not supported on this platform");
    1
}
