use std::process::Command;

use clap::{Parser, Subcommand};

use crate::config;

const PLIST_LABEL: &str = "dev.bollyai.bolly";

#[derive(Parser)]
#[command(
    name = "bolly",
    about = "Bolly — AI companion",
    version = env!("CARGO_PKG_VERSION"),
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

#[derive(Subcommand)]
pub enum CliCommand {
    /// Start the Bolly background service
    Start,
    /// Stop the Bolly background service
    Stop,
    /// Restart the Bolly background service
    Restart,
    /// Show service status
    Status,
    /// Stream logs in real time
    Logs,
    /// Print version
    Version,
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
            println!("bolly {}", env!("CARGO_PKG_VERSION"));
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
        eprintln!("plist not found at {plist} — was Bolly installed with the install script?");
        return 1;
    }
    let domain = format!("gui/{}", uid());
    let status = Command::new("launchctl")
        .args(["bootstrap", &domain, &plist])
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("Bolly service started.");
            0
        }
        Ok(s) => {
            // exit code 37 = already loaded
            if s.code() == Some(37) {
                println!("Bolly service is already running.");
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
            println!("Bolly service stopped.");
            0
        }
        Ok(s) => {
            if s.code() == Some(3) {
                println!("Bolly service is not running.");
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
                println!("Bolly is running (pid {pid}, state: {state})");
            } else {
                println!("Bolly is not running.");
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
    let log_path = config::workspace_root().join("bolly.log");
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
    run_systemctl(&["start", "bolly"], "started")
}

#[cfg(target_os = "linux")]
fn svc_stop() -> i32 {
    run_systemctl(&["stop", "bolly"], "stopped")
}

#[cfg(target_os = "linux")]
fn svc_status() -> i32 {
    let output = Command::new("systemctl")
        .args(["is-active", "bolly"])
        .output();
    match output {
        Ok(out) => {
            let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if state == "active" {
                println!("Bolly is running.");
            } else {
                println!("Bolly is not running ({state}).");
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
        .args(["-u", "bolly", "-f", "--no-pager"])
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
            println!("Bolly service {verb}.");
            return 0;
        }
    }
    let system = Command::new("sudo").arg("systemctl").args(args).status();
    match system {
        Ok(s) if s.success() => {
            println!("Bolly service {verb}.");
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
