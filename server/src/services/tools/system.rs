use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use crate::services::chat;
use crate::services::tool::{Tool, ToolDefinition};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::broadcast;

use super::companion::load_mood_state;
use super::{ToolExecError, openai_schema};
use crate::app::state::PendingSecret;
use crate::domain::events::ServerEvent;

// ---------------------------------------------------------------------------
// run_command
// ---------------------------------------------------------------------------

pub struct RunCommandTool {
    instance_dir: PathBuf,
    events: broadcast::Sender<ServerEvent>,
    instance_slug: String,
    chat_id: String,
    github_token: Option<String>,
}

impl RunCommandTool {
    pub fn new(
        workspace_dir: &Path,
        instance_slug: &str,
        chat_id: &str,
        events: broadcast::Sender<ServerEvent>,
        github_token: Option<String>,
    ) -> Self {
        Self {
            instance_dir: workspace_dir.join("instances").join(instance_slug),
            events,
            instance_slug: instance_slug.to_string(),
            chat_id: chat_id.to_string(),
            github_token,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct RunCommandArgs {
    /// The shell command to execute.
    pub command: String,
    /// Working directory for the command. Absolute path (e.g. "/Users/timur/projects/app"). Default: instance directory.
    pub cwd: Option<String>,
    /// Timeout in seconds. Choose based on what the command does: quick commands (ls, cat, echo) use 5-10, builds/installs use 60-120, long tasks up to 300. Default: 30.
    pub timeout_secs: Option<u64>,
    /// Allocate a pseudo-terminal (PTY) for this command. Enables commands that require a TTY (ssh, gh auth, python REPL, etc.). Default: true.
    pub pty: Option<bool>,
}

impl Tool for RunCommandTool {
    const NAME: &'static str = "run_command";
    type Error = ToolExecError;
    type Args = RunCommandArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "run_command".into(),
            description:
                "Run a shell command. Prefer built-in tools when available (github_* for git, \
                edit_file for editing, web_fetch for HTTP). Use run_command for everything else: \
                builds, tests, scripts, system commands."
                    .into(),
            parameters: openai_schema::<RunCommandArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let command = args.command.trim().to_string();
        if command.is_empty() {
            return Err(ToolExecError("command cannot be empty".into()));
        }

        let work_dir = args
            .cwd
            .as_deref()
            .filter(|p| p.starts_with('/'))
            .map(PathBuf::from)
            .unwrap_or_else(|| self.instance_dir.clone());

        let timeout = args.timeout_secs.unwrap_or(30).min(1800);
        let use_pty = args.pty.unwrap_or(true);

        log::info!(
            "[run_command] executing: {} (cwd: {}, pty: {})",
            command,
            work_dir.display(),
            use_pty
        );

        let github_token = self.github_token.clone();

        if use_pty {
            let cmd = command.clone();
            let dir = work_dir.clone();
            let events = self.events.clone();
            let instance_slug = self.instance_slug.clone();
            let chat_id = self.chat_id.clone();
            let chunk_cb: Box<dyn Fn(&str) + Send> = Box::new(move |chunk: &str| {
                let redacted = super::redact_secrets(chunk);
                let _ = events.send(ServerEvent::ToolOutputChunk {
                    instance_slug: instance_slug.clone(),
                    chat_id: chat_id.clone(),
                    chunk: redacted,
                });
            });
            let env_pairs: Vec<(String, String)> = if let Some(ref t) = github_token {
                vec![
                    ("GITHUB_TOKEN".into(), t.clone()),
                    ("GH_TOKEN".into(), t.clone()),
                ]
            } else {
                vec![]
            };
            let result = tokio::task::spawn_blocking(move || {
                let env_refs: Vec<(&str, &str)> = env_pairs
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                run_command_pty(&cmd, &dir, timeout, Some(&chunk_cb), &env_refs)
            })
            .await
            .map_err(|e| ToolExecError(format!("task join error: {e}")))?
            .map_err(|e| ToolExecError(e))?;

            match result {
                PtyRunResult::Completed(output) => Ok(output),
                PtyRunResult::WaitingForInput { output, session_id } => {
                    let mut msg = String::new();
                    msg.push_str("Command is waiting for interactive input.\n\nOutput so far:\n");
                    msg.push_str(&output);
                    msg.push_str(&format!(
                        "\n\nTo respond to this prompt, use the interactive_session tool:\n\
                         - action: \"write\"\n\
                         - session_id: \"{session_id}\"\n\
                         - input: \"<your response>\\n\"\n\n\
                         After you're done, close with action: \"close\", session_id: \"{session_id}\""
                    ));
                    Ok(msg)
                }
            }
        } else {
            let mut cmd = tokio::process::Command::new("sh");
            cmd.arg("-c")
                .arg(&command)
                .current_dir(&work_dir)
                .stdin(std::process::Stdio::null());
            if let Some(ref token) = github_token {
                cmd.env("GITHUB_TOKEN", token).env("GH_TOKEN", token);
            }
            let output =
                tokio::time::timeout(std::time::Duration::from_secs(timeout), cmd.output())
                    .await
                    .map_err(|_| {
                        ToolExecError(format!("command timed out after {timeout}s: {command}"))
                    })?
                    .map_err(|e| ToolExecError(format!("failed to execute command: {e}")))?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let mut result = String::new();
            if !stdout.is_empty() {
                let truncated: String = stdout.chars().take(4000).collect();
                result.push_str(&truncated);
                if stdout.len() > 4000 {
                    result.push_str("\n...(output truncated)");
                }
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                let truncated: String = stderr.chars().take(2000).collect();
                result.push_str(&format!("stderr: {truncated}"));
            }

            if result.is_empty() {
                result = format!(
                    "command completed with exit code {}",
                    output.status.code().unwrap_or(-1)
                );
            }

            Ok(result)
        }
    }
}

/// Result from PTY command execution.
enum PtyRunResult {
    /// Command completed normally.
    Completed(String),
    /// Command is waiting for interactive input; PTY parked as a session.
    WaitingForInput { output: String, session_id: String },
}

/// Check if the last few lines of output look like an interactive prompt.
fn looks_like_interactive_prompt(output: &str) -> bool {
    // Check last few non-empty lines (prompts may have trailing blank lines or box-drawing chars)
    let lines: Vec<&str> = output.lines().rev().take(5).collect();
    for raw_line in &lines {
        let line = raw_line.trim();
        if line.is_empty() || line.chars().all(|c| "│┃|".contains(c)) {
            continue;
        }
        // Standard prompt patterns
        if line.ends_with("? ")
            || line.ends_with(": ")
            || line.ends_with("> ")
            || line.ends_with("] ")
            || line.ends_with("› ")
            || line.contains("(y/n)")
            || line.contains("[Y/n]")
            || line.contains("[y/N]")
            || line.to_lowercase().contains("password")
        {
            return true;
        }
        // @clack/prompts style: "◆  Question text" or "◇  Question text"
        if line.starts_with('◆') || line.starts_with('◇') || line.starts_with('●') {
            return true;
        }
        // inquirer/prompts style: "? Question (option)" or "❯ Option"
        if line.starts_with("? ") || line.starts_with("❯ ") || line.starts_with("❮ ") {
            return true;
        }
        // Line ends with a cursor placeholder (common in TUI prompts)
        if line.ends_with('_') || line.ends_with("▌") || line.ends_with("█") {
            return true;
        }
        break; // Only check the first meaningful line from the end
    }
    false
}

/// Execute a command inside a pseudo-terminal (PTY).
/// If the command waits for interactive input, the PTY is parked as a session
/// and the caller is told to use `interactive_session` to continue.
fn run_command_pty(
    command: &str,
    work_dir: &Path,
    timeout_secs: u64,
    on_chunk: Option<&dyn Fn(&str)>,
    env_vars: &[(&str, &str)],
) -> Result<PtyRunResult, String> {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use std::io::Read;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    // Prune stale sessions (older than 5 minutes) on each run_command call
    if let Ok(mut sessions) = PTY_SESSIONS.lock() {
        sessions.retain(|_, _| true); // TODO: add created_at to PtySession for TTL
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("failed to open pty: {e}"))?;

    let mut cmd = CommandBuilder::new("sh");
    cmd.args(["-c", command]);
    cmd.cwd(work_dir);
    for (k, v) in env_vars {
        cmd.env(k, v);
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("failed to spawn command: {e}"))?;

    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("failed to clone pty reader: {e}"))?;

    // Keep the writer alive — we may need it if the command is interactive
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("failed to take pty writer: {e}"))?;

    // Use Vec<u8> channel (compatible with PtySession) — empty vec signals EOF
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(Vec::new());
                    break;
                }
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = tx.send(Vec::new());
                    break;
                }
            }
        }
    });

    let child_pid = child.process_id();
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut output = Vec::new();
    let max_capture = 6000usize;
    let idle_check_interval = Duration::from_secs(3);
    let mut last_data_at = Instant::now();

    // Track whether we should park as interactive session
    let mut park_as_session = false;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let _ = child.kill();
            return Err(format!("command timed out after {timeout_secs}s"));
        }

        let wait = remaining.min(idle_check_interval);

        match rx.recv_timeout(wait) {
            Ok(data) if data.is_empty() => break, // EOF
            Ok(data) => {
                let raw = String::from_utf8_lossy(&data);
                let clean = strip_ansi_codes(&raw);
                let has_content = !clean.trim().is_empty();
                if has_content {
                    if let Some(cb) = &on_chunk {
                        cb(&clean);
                    }
                }
                output.extend_from_slice(&data);
                // Only reset idle timer for meaningful data (not just cursor/spinner ANSI codes)
                if has_content {
                    last_data_at = Instant::now();
                }
                if output.len() > max_capture {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !output.is_empty() && last_data_at.elapsed() >= idle_check_interval {
                    let waiting_on_tty =
                        child_pid.map_or(false, |pid| is_process_tree_waiting_on_tty(pid));
                    let raw = String::from_utf8_lossy(&output);
                    let clean = strip_ansi_codes(&raw);
                    let prompt_detected = looks_like_interactive_prompt(&clean);

                    if waiting_on_tty || prompt_detected {
                        park_as_session = true;
                        break;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let raw = String::from_utf8_lossy(&output);
    let clean = strip_ansi_codes(&raw);
    let truncated: String = clean.chars().take(4000).collect();

    if park_as_session {
        // Park the PTY into the session pool so the LLM can interact with it
        let session_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let session = PtySession {
            child,
            writer,
            output_rx: rx,
        };
        PTY_SESSIONS
            .lock()
            .unwrap()
            .insert(session_id.clone(), session);
        log::info!("[run_command] parked interactive session: {session_id}");
        return Ok(PtyRunResult::WaitingForInput {
            output: truncated,
            session_id,
        });
    }

    // Normal completion — drop writer, wait for exit
    drop(writer);
    let exit_status = child.wait().ok();

    if truncated.is_empty() {
        let code = exit_status
            .map(|s| s.exit_code().to_string())
            .unwrap_or_else(|| "unknown".into());
        Ok(PtyRunResult::Completed(format!(
            "command completed with exit code {code}"
        )))
    } else {
        let mut result = truncated;
        if clean.chars().count() > 4000 {
            result.push_str("\n...(output truncated)");
        }
        Ok(PtyRunResult::Completed(result))
    }
}

/// Check whether any process in the tree rooted at `pid` is blocked waiting
/// for TTY input.  On Linux we inspect `/proc/PID/wchan` — when a process is
/// blocked in the kernel's TTY line-discipline read path, wchan shows
/// `n_tty_read` (or `wait_woken` which is called from within it).
///
/// We walk the whole process tree because `sh -c "some_command"` means the
/// shell is the direct child but the *actual* interactive process is a
/// grandchild.
///
/// On non-Linux platforms this always returns false (falls back to timeout).
#[cfg(target_os = "linux")]
fn is_process_tree_waiting_on_tty(root_pid: u32) -> bool {
    // Collect all descendant PIDs via /proc/*/stat ppid field.
    let mut pids = vec![root_pid];
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                if let Ok(pid) = name.parse::<u32>() {
                    if pid == root_pid {
                        continue;
                    }
                    // Read ppid (field 4) from /proc/PID/stat
                    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                        // Fields after comm (which is in parens and may contain spaces)
                        if let Some(after_comm) = stat.rfind(')') {
                            let fields: Vec<&str> =
                                stat[after_comm + 2..].split_whitespace().collect();
                            // field[0] = state, field[1] = ppid
                            if let Some(ppid_str) = fields.get(1) {
                                if let Ok(ppid) = ppid_str.parse::<u32>() {
                                    if pids.contains(&ppid) {
                                        pids.push(pid);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Check wchan for each process in the tree.
    for pid in &pids {
        if let Ok(wchan) = std::fs::read_to_string(format!("/proc/{pid}/wchan")) {
            let wchan = wchan.trim();
            if wchan == "n_tty_read" || wchan == "wait_woken" {
                return true;
            }
        }
    }

    false
}

#[cfg(not(target_os = "linux"))]
fn is_process_tree_waiting_on_tty(_root_pid: u32) -> bool {
    false
}

/// Strip ANSI escape sequences from a string.
fn strip_ansi_codes(s: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]|\x1b\].*?\x07|\x1b\[.*?[mGKHJ]|\r").unwrap();
    re.replace_all(s, "").into_owned()
}

// ---------------------------------------------------------------------------
// interactive_session — persistent PTY sessions
// ---------------------------------------------------------------------------

struct PtySession {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn std::io::Write + Send>,
    output_rx: std::sync::mpsc::Receiver<Vec<u8>>,
}

impl PtySession {
    fn drain_output(&self, timeout: std::time::Duration) -> String {
        let mut output = Vec::new();
        match self.output_rx.recv_timeout(timeout) {
            Ok(data) => output.extend(data),
            Err(_) => {}
        }
        while let Ok(data) = self.output_rx.try_recv() {
            output.extend(data);
            if output.len() > 8000 {
                break;
            }
        }
        let raw = String::from_utf8_lossy(&output);
        strip_ansi_codes(&raw)
    }
}

static PTY_SESSIONS: LazyLock<Mutex<HashMap<String, PtySession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub struct InteractiveSessionTool {
    instance_dir: PathBuf,
}

impl InteractiveSessionTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            instance_dir: workspace_dir.join("instances").join(instance_slug),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct InteractiveSessionArgs {
    /// Action to perform: "start", "write", "read", or "close".
    pub action: String,
    /// Shell command to execute (required for "start").
    pub command: Option<String>,
    /// Working directory, absolute path (for "start"). Default: instance directory.
    pub cwd: Option<String>,
    /// Session ID (required for "write", "read", "close"). Returned by "start".
    pub session_id: Option<String>,
    /// Input to send to the process (for "write"). Supports escape sequences:
    /// use \n for Enter/Return, \t for Tab. For arrow keys: \x1b[A (up), \x1b[B (down),
    /// \x1b[C (right), \x1b[D (left). For Ctrl+C: \x03, Ctrl+D: \x04.
    pub input: Option<String>,
    /// Seconds to wait for output after starting or writing. Default: 2.
    pub wait_secs: Option<u64>,
}

impl Tool for InteractiveSessionTool {
    const NAME: &'static str = "interactive_session";
    type Error = ToolExecError;
    type Args = InteractiveSessionArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "interactive_session".into(),
            description:
                "Persistent interactive terminal session. Actions: start, write, read, close."
                    .into(),
            parameters: openai_schema::<InteractiveSessionArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match args.action.as_str() {
            "start" => {
                let command = args.command.ok_or_else(|| {
                    ToolExecError("\"command\" is required for action \"start\"".into())
                })?;

                let work_dir = args
                    .cwd
                    .as_deref()
                    .filter(|p| p.starts_with('/'))
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.instance_dir.clone());

                let wait = std::time::Duration::from_secs(args.wait_secs.unwrap_or(2));
                let session_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

                log::info!(
                    "[interactive_session] starting: {} (session: {})",
                    command,
                    session_id
                );

                let cmd = command.clone();
                let dir = work_dir.clone();
                let sid = session_id.clone();

                let initial_output =
                    tokio::task::spawn_blocking(move || -> Result<String, String> {
                        use portable_pty::{CommandBuilder, PtySize, native_pty_system};

                        let pty_system = native_pty_system();
                        let pair = pty_system
                            .openpty(PtySize {
                                rows: 24,
                                cols: 120,
                                pixel_width: 0,
                                pixel_height: 0,
                            })
                            .map_err(|e| format!("failed to open pty: {e}"))?;

                        let mut pty_cmd = CommandBuilder::new("sh");
                        pty_cmd.args(["-c", &cmd]);
                        pty_cmd.cwd(&dir);

                        let child = pair
                            .slave
                            .spawn_command(pty_cmd)
                            .map_err(|e| format!("failed to spawn command: {e}"))?;

                        drop(pair.slave);

                        let mut reader = pair
                            .master
                            .try_clone_reader()
                            .map_err(|e| format!("failed to clone pty reader: {e}"))?;

                        let writer = pair
                            .master
                            .take_writer()
                            .map_err(|e| format!("failed to take pty writer: {e}"))?;

                        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
                        std::thread::spawn(move || {
                            let mut buf = vec![0u8; 4096];
                            loop {
                                match std::io::Read::read(&mut reader, &mut buf) {
                                    Ok(0) => {
                                        let _ = tx.send(Vec::new());
                                        break;
                                    }
                                    Ok(n) => {
                                        if tx.send(buf[..n].to_vec()).is_err() {
                                            break;
                                        }
                                    }
                                    Err(_) => {
                                        let _ = tx.send(Vec::new());
                                        break;
                                    }
                                }
                            }
                        });

                        let session = PtySession {
                            child,
                            writer,
                            output_rx: rx,
                        };

                        let initial = session.drain_output(wait);

                        PTY_SESSIONS.lock().unwrap().insert(sid, session);

                        Ok(initial)
                    })
                    .await
                    .map_err(|e| ToolExecError(format!("task join error: {e}")))?
                    .map_err(|e| ToolExecError(e))?;

                let mut result = format!("Session started: {session_id}\n");
                if !initial_output.is_empty() {
                    result.push_str(&initial_output);
                } else {
                    result.push_str("(waiting for output...)");
                }
                Ok(result)
            }

            "write" => {
                let session_id = args.session_id.ok_or_else(|| {
                    ToolExecError("\"session_id\" is required for action \"write\"".into())
                })?;
                let input = args.input.ok_or_else(|| {
                    ToolExecError("\"input\" is required for action \"write\"".into())
                })?;
                let wait = std::time::Duration::from_secs(args.wait_secs.unwrap_or(2));

                let bytes = unescape_input(&input);

                let sid = session_id.clone();
                let output = tokio::task::spawn_blocking(move || -> Result<String, String> {
                    let mut sessions = PTY_SESSIONS.lock().unwrap();
                    let session = sessions.get_mut(&sid).ok_or_else(|| {
                        format!("no session with id \"{sid}\". It may have been closed or expired.")
                    })?;

                    let _ = session.drain_output(std::time::Duration::from_millis(100));

                    std::io::Write::write_all(&mut session.writer, &bytes)
                        .map_err(|e| format!("write error: {e}"))?;
                    std::io::Write::flush(&mut session.writer)
                        .map_err(|e| format!("flush error: {e}"))?;

                    let output = session.drain_output(wait);
                    Ok(output)
                })
                .await
                .map_err(|e| ToolExecError(format!("task join error: {e}")))?
                .map_err(|e| ToolExecError(e))?;

                if output.is_empty() {
                    Ok(format!(
                        "[session {session_id}] Input sent. No new output yet."
                    ))
                } else {
                    Ok(format!("[session {session_id}]\n{output}"))
                }
            }

            "read" => {
                let session_id = args.session_id.ok_or_else(|| {
                    ToolExecError("\"session_id\" is required for action \"read\"".into())
                })?;
                let wait = std::time::Duration::from_secs(args.wait_secs.unwrap_or(2));

                let sid = session_id.clone();
                let output = tokio::task::spawn_blocking(move || -> Result<String, String> {
                    let sessions = PTY_SESSIONS.lock().unwrap();
                    let session = sessions
                        .get(&sid)
                        .ok_or_else(|| format!("no session with id \"{sid}\""))?;
                    Ok(session.drain_output(wait))
                })
                .await
                .map_err(|e| ToolExecError(format!("task join error: {e}")))?
                .map_err(|e| ToolExecError(e))?;

                if output.is_empty() {
                    Ok(format!("[session {session_id}] No new output."))
                } else {
                    Ok(format!("[session {session_id}]\n{output}"))
                }
            }

            "close" => {
                let session_id = args.session_id.ok_or_else(|| {
                    ToolExecError("\"session_id\" is required for action \"close\"".into())
                })?;

                let sid = session_id.clone();
                let output = tokio::task::spawn_blocking(move || -> Result<String, String> {
                    let mut sessions = PTY_SESSIONS.lock().unwrap();
                    if let Some(mut session) = sessions.remove(&sid) {
                        let final_output =
                            session.drain_output(std::time::Duration::from_millis(500));
                        let _ = session.child.kill();
                        let _ = session.child.wait();
                        Ok(final_output)
                    } else {
                        Ok(String::new())
                    }
                })
                .await
                .map_err(|e| ToolExecError(format!("task join error: {e}")))?
                .map_err(|e| ToolExecError(e))?;

                let mut result = format!("Session {session_id} closed.");
                if !output.is_empty() {
                    result.push_str(&format!("\nFinal output:\n{output}"));
                }
                Ok(result)
            }

            other => Err(ToolExecError(format!(
                "unknown action \"{other}\". Use \"start\", \"write\", \"read\", or \"close\"."
            ))),
        }
    }
}

fn unescape_input(s: &str) -> Vec<u8> {
    let mut result = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    result.push(b'\n');
                }
                Some('r') => {
                    chars.next();
                    result.push(b'\r');
                }
                Some('t') => {
                    chars.next();
                    result.push(b'\t');
                }
                Some('\\') => {
                    chars.next();
                    result.push(b'\\');
                }
                Some('x') => {
                    chars.next();
                    let mut hex = String::new();
                    for _ in 0..2 {
                        if let Some(&h) = chars.peek() {
                            if h.is_ascii_hexdigit() {
                                hex.push(h);
                                chars.next();
                            }
                        }
                    }
                    if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                        result.push(byte);
                    }
                }
                _ => result.push(b'\\'),
            }
        } else {
            let mut buf = [0u8; 4];
            result.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    }
    result
}

// ---------------------------------------------------------------------------
// get_settings
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// get_time
// ---------------------------------------------------------------------------

pub struct GetTimeTool {
    instance_dir: PathBuf,
}

impl GetTimeTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            instance_dir: workspace_dir.join("instances").join(instance_slug),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct GetTimeArgs {}

impl Tool for GetTimeTool {
    const NAME: &'static str = "get_time";
    type Error = ToolExecError;
    type Args = GetTimeArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "get_time".into(),
            description: "Get the current date and time in the user's timezone.".into(),
            parameters: openai_schema::<GetTimeArgs>(),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let now = crate::routes::instances::format_instance_now(&self.instance_dir);
        Ok(now)
    }
}

// ---------------------------------------------------------------------------
// get_settings
// ---------------------------------------------------------------------------

pub struct GetSettingsTool {
    config_path: PathBuf,
    workspace_dir: PathBuf,
    instance_slug: String,
    instance_dir: PathBuf,
    google: Option<crate::services::google::GoogleClient>,
}

impl GetSettingsTool {
    pub fn new(
        config_path: &Path,
        workspace_dir: &Path,
        instance_slug: &str,
        google: Option<crate::services::google::GoogleClient>,
    ) -> Self {
        Self {
            config_path: config_path.to_path_buf(),
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
            instance_dir: workspace_dir.join("instances").join(instance_slug),
            google,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct GetSettingsArgs {}

impl Tool for GetSettingsTool {
    const NAME: &'static str = "get_settings";
    type Error = ToolExecError;
    type Args = GetSettingsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "get_settings".into(),
            description: "Get all current settings and status: companion name, timezone, LLM model, connected accounts (Google, email, GitHub), MCP servers, mood.".into(),
            parameters: openai_schema::<GetSettingsArgs>(),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let mut lines = Vec::new();

        // Companion name
        let state_path = self.instance_dir.join("project_state.json");
        let project_state: serde_json::Value = fs::read_to_string(&state_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| serde_json::json!({}));

        let name = project_state
            .get("identity")
            .and_then(|i| i.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("(not set)");
        lines.push(format!("companion name: {name}"));

        // Timezone
        let tz = project_state
            .get("timezone")
            .and_then(|t| t.as_str())
            .unwrap_or("UTC (default)");
        lines.push(format!("timezone: {tz}"));

        // Mood
        let mood = load_mood_state(&self.instance_dir);
        lines.push(format!("mood: {}", mood.companion_mood));

        // LLM
        if let Ok(raw) = fs::read_to_string(&self.config_path) {
            if let Ok(config) = toml::from_str::<crate::config::Config>(&raw) {
                let mode = match config.llm.model_mode {
                    crate::config::ModelMode::Auto => "auto",
                    crate::config::ModelMode::Fast => "fast",
                    crate::config::ModelMode::Heavy => "heavy",
                };
                if let Some(reason) = config.llm.setup_required() {
                    lines.push(format!(
                        "llm: {:?} — setup required: {reason}",
                        config.llm.provider
                    ));
                } else {
                    lines.push(format!(
                        "llm: {:?} / {} (mode: {mode})",
                        config.llm.provider,
                        config.llm.model_name()
                    ));
                }
                if config.llm.provider != crate::config::LlmProvider::Codex {
                    lines.push(format!("fast model: {}", config.llm.fast_model_name()));
                }

                let keys = config.llm.configured_providers();
                if keys.is_empty() {
                    lines.push("api keys: none configured".into());
                } else {
                    lines.push(format!("api keys: {}", keys.join(", ")));
                }

                // GitHub
                // GitHub — check instance config first, then fall back to global
                let instance_cfg =
                    crate::config::InstanceConfig::load(&self.workspace_dir, &self.instance_slug);
                let github_token_set =
                    !instance_cfg.github.token.is_empty() || !config.github.token.is_empty();
                if github_token_set {
                    lines.push("github: token configured".into());
                } else {
                    lines.push("github: not connected".into());
                }

                // Screen recording
                lines.push(format!(
                    "screen recording: {}",
                    if instance_cfg.screen_recording {
                        "enabled — recording user's screen between heartbeats"
                    } else {
                        "disabled (off by default — enable via update_config to observe the user's screen and offer contextual help)"
                    }
                ));

                // MCP servers
                if config.mcp_servers.is_empty() {
                    lines.push("extensions (mcp): none".into());
                } else {
                    let names: Vec<&str> =
                        config.mcp_servers.iter().map(|s| s.name.as_str()).collect();
                    lines.push(format!("extensions (mcp): {}", names.join(", ")));
                }
            }
        }

        // Google accounts
        let google_accounts = if let Some(ref g) = self.google {
            g.accounts(&self.instance_slug).await.unwrap_or_default()
        } else {
            vec![]
        };
        if google_accounts.is_empty() {
            lines.push("google accounts: none connected".into());
        } else {
            let emails: Vec<&str> = google_accounts.iter().map(|a| a.email.as_str()).collect();
            lines.push(format!("google accounts: {}", emails.join(", ")));
        }

        // Email accounts (SMTP/IMAP)
        let email_accounts =
            crate::config::EmailAccounts::load(&self.workspace_dir, &self.instance_slug);
        if email_accounts.is_empty() {
            lines.push("email accounts (smtp/imap): none configured".into());
        } else {
            let emails: Vec<String> = email_accounts
                .iter()
                .map(|a| {
                    let addr = if a.smtp_from.is_empty() {
                        &a.smtp_user
                    } else {
                        &a.smtp_from
                    };
                    addr.to_string()
                })
                .collect();
            lines.push(format!("email accounts (smtp/imap): {}", emails.join(", ")));
        }

        // Soul
        let soul_exists = self.instance_dir.join("soul.md").exists();
        lines.push(format!(
            "soul.md: {}",
            if soul_exists { "exists" } else { "not created" }
        ));

        Ok(lines.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// update_config
// ---------------------------------------------------------------------------

pub struct UpdateConfigTool {
    config_path: PathBuf,
    workspace_dir: PathBuf,
    instance_slug: String,
    instance_dir: PathBuf,
    machine_registry: crate::services::machine_registry::MachineRegistry,
}

impl UpdateConfigTool {
    pub fn new(
        config_path: &Path,
        workspace_dir: &Path,
        instance_slug: &str,
        machine_registry: crate::services::machine_registry::MachineRegistry,
    ) -> Self {
        Self {
            config_path: config_path.to_path_buf(),
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
            instance_dir: workspace_dir.join("instances").join(instance_slug),
            machine_registry,
        }
    }
}

/// Arguments for update_config tool.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateConfigArgs {
    /// OpenAI API key. Leave null to keep current.
    pub openai_key: Option<String>,
    /// Anthropic API key. Leave null to keep current.
    pub anthropic_key: Option<String>,
    /// Brave Search API key. Leave null to keep current.
    pub brave_search_key: Option<String>,
    /// Add an MCP server. Provide as {"name": "...", "url": "..."}. Leave null to skip.
    pub add_mcp_server: Option<McpServerArg>,
    /// Remove an MCP server by name. Leave null to skip.
    pub remove_mcp_server: Option<String>,
    /// Set the user's timezone (IANA format, e.g. "Asia/Bishkek", "Europe/Moscow"). Leave null to keep current.
    pub timezone: Option<String>,
    /// Set the companion's display name. Leave null to keep current.
    pub companion_name: Option<String>,
    /// Set the GitHub personal access token. Leave null to keep current.
    pub github_token: Option<String>,
    /// Add an email account (SMTP/IMAP). Provide as {"smtp_host": "...", "smtp_port": 587, "smtp_user": "...", "smtp_password": "...", "smtp_from": "...", "imap_host": "...", "imap_port": 993, "imap_user": "...", "imap_password": "..."}.
    pub add_email_account: Option<EmailAccountArg>,
    /// Remove an email account by address (matches smtp_from or smtp_user).
    pub remove_email_account: Option<String>,
    /// Model routing mode: "auto" (classifier picks fast/heavy per message), "fast" (always cheap model), "heavy" (always powerful model). Leave null to keep current.
    pub model_mode: Option<String>,
    /// Enable/disable screen recording observation. When true, the companion records the user's
    /// screen between heartbeats and analyzes it to offer contextual suggestions. Default: false (off).
    pub screen_recording: Option<bool>,
    /// Update a child agent's interval. Provide as {"agent_name": "companion", "interval_hours": 0.5}.
    /// Set interval_hours to 0 to make it on-demand only. Changes take effect on next server restart.
    pub agent_interval: Option<AgentIntervalArg>,
    /// Reset a built-in agent to its default config. Provide the agent name (e.g. "companion", "reflection", "observer").
    /// Only works for built-in agents, not custom ones.
    pub reset_agent: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct AgentIntervalArg {
    /// Name of the child agent to update.
    pub agent_name: String,
    /// New interval in hours. 0 = on-demand only, 0.25 = every 15 min, 1 = every hour, etc.
    pub interval_hours: f64,
}

#[derive(Deserialize, JsonSchema)]
pub struct EmailAccountArg {
    pub smtp_host: String,
    #[serde(default = "default_587")]
    pub smtp_port: u16,
    pub smtp_user: String,
    pub smtp_password: String,
    pub smtp_from: String,
    #[serde(default)]
    pub imap_host: String,
    #[serde(default = "default_993")]
    pub imap_port: u16,
    #[serde(default)]
    pub imap_user: String,
    #[serde(default)]
    pub imap_password: String,
}

fn default_587() -> u16 {
    587
}
fn default_993() -> u16 {
    993
}

#[derive(Deserialize, JsonSchema)]
pub struct McpServerArg {
    /// Human-readable name for the MCP server (e.g. "excalidraw").
    pub name: String,
    /// HTTP URL of the MCP server (e.g. "https://mcp.excalidraw.com/mcp").
    pub url: String,
}

impl Tool for UpdateConfigTool {
    const NAME: &'static str = "update_config";
    type Error = ToolExecError;
    type Args = UpdateConfigArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "update_config".into(),
            description: "Update config and instance settings (API keys, MCP servers, timezone, companion name, GitHub token). Only provided fields change.".into(),
            parameters: openai_schema::<UpdateConfigArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let raw = fs::read_to_string(&self.config_path)
            .map_err(|e| ToolExecError(format!("failed to read config: {e}")))?;
        let mut config: crate::config::Config = toml::from_str(&raw)
            .map_err(|e| ToolExecError(format!("failed to parse config: {e}")))?;

        let mut changes = Vec::new();

        if let Some(mode) = &args.model_mode {
            let m = mode.trim().to_lowercase();
            match m.as_str() {
                "auto" => config.llm.model_mode = crate::config::ModelMode::Auto,
                "fast" => config.llm.model_mode = crate::config::ModelMode::Fast,
                "heavy" => config.llm.model_mode = crate::config::ModelMode::Heavy,
                other => {
                    return Err(ToolExecError(format!(
                        "unknown model_mode \"{other}\". supported: auto, fast, heavy"
                    )));
                }
            }
            changes.push(format!("model_mode → {m}"));
        }

        if let Some(key) = &args.openai_key {
            let k = key.trim().to_string();
            config.llm.tokens.open_ai = k.clone();
            changes.push(if k.is_empty() {
                "openai key cleared".into()
            } else {
                "openai key updated".into()
            });
        }

        if let Some(key) = &args.anthropic_key {
            let k = key.trim().to_string();
            config.llm.tokens.anthropic = k.clone();
            changes.push(if k.is_empty() {
                "anthropic key cleared".into()
            } else {
                "anthropic key updated".into()
            });
        }

        if let Some(key) = &args.brave_search_key {
            let k = key.trim().to_string();
            config.llm.tokens.brave_search = k.clone();
            changes.push(if k.is_empty() {
                "brave search key cleared".into()
            } else {
                "brave search key updated".into()
            });
        }

        if let Some(server) = &args.add_mcp_server {
            let name = server.name.trim().to_string();
            let url = server.url.trim().to_string();
            if name.is_empty() || url.is_empty() {
                return Err(ToolExecError(
                    "MCP server name and url cannot be empty".into(),
                ));
            }
            if config.mcp_servers.iter().any(|s| s.name == name) {
                return Err(ToolExecError(format!("MCP server '{name}' already exists")));
            }
            config.mcp_servers.push(crate::config::McpServerConfig {
                name: name.clone(),
                url: Some(url),
                command: None,
                args: Default::default(),
                headers: Default::default(),
            });
            changes.push(format!("added MCP server '{name}'"));
        }

        if let Some(name) = &args.remove_mcp_server {
            let name = name.trim().to_string();
            let before = config.mcp_servers.len();
            config.mcp_servers.retain(|s| s.name != name);
            if config.mcp_servers.len() == before {
                return Err(ToolExecError(format!("MCP server '{name}' not found")));
            }
            changes.push(format!("removed MCP server '{name}'"));
        }

        // --- Instance-specific settings (project_state.json) ---
        let mut instance_changes = false;

        if args.timezone.is_some() || args.companion_name.is_some() {
            let state_path = self.instance_dir.join("project_state.json");
            let mut project_state: serde_json::Value = fs::read_to_string(&state_path)
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_else(|| serde_json::json!({}));

            if let Some(tz) = &args.timezone {
                let tz = tz.trim().to_string();
                if !tz.is_empty() {
                    if tz.parse::<chrono_tz::Tz>().is_err() {
                        return Err(ToolExecError(format!(
                            "invalid timezone \"{tz}\". use IANA format like \"Asia/Bishkek\""
                        )));
                    }
                }
                project_state["timezone"] = serde_json::Value::String(tz.clone());
                changes.push(format!(
                    "timezone → {}",
                    if tz.is_empty() { "UTC" } else { &tz }
                ));
                instance_changes = true;
            }

            if let Some(name) = &args.companion_name {
                let name = name.trim().to_string();
                if project_state.get("identity").is_none() {
                    project_state["identity"] = serde_json::json!({});
                }
                project_state["identity"]["name"] = serde_json::Value::String(name.clone());
                changes.push(format!("companion name → {name}"));
                instance_changes = true;
            }

            if instance_changes {
                fs::create_dir_all(&self.instance_dir).ok();
                let body = serde_json::to_string_pretty(&project_state)
                    .map_err(|e| ToolExecError(format!("failed to serialize state: {e}")))?;
                fs::write(&state_path, body)
                    .map_err(|e| ToolExecError(format!("failed to write state: {e}")))?;
            }
        }

        if let Some(token) = &args.github_token {
            let token = token.trim().to_string();
            // Write github token to per-instance config, not global
            let mut instance_cfg =
                crate::config::InstanceConfig::load(&self.workspace_dir, &self.instance_slug);
            instance_cfg.github.token = token.clone();
            instance_cfg
                .save(&self.workspace_dir, &self.instance_slug)
                .map_err(|e| ToolExecError(format!("failed to save instance config: {e}")))?;
            changes.push(if token.is_empty() {
                "github token removed".into()
            } else {
                "github token updated".into()
            });
        }

        if let Some(enabled) = args.screen_recording {
            let mut instance_cfg =
                crate::config::InstanceConfig::load(&self.workspace_dir, &self.instance_slug);
            instance_cfg.screen_recording = enabled;
            instance_cfg
                .save(&self.workspace_dir, &self.instance_slug)
                .map_err(|e| ToolExecError(format!("failed to save instance config: {e}")))?;
            changes.push(format!(
                "screen recording → {}",
                if enabled { "enabled" } else { "disabled" }
            ));

            // Enable/disable the observer agent
            let observer_path = self
                .workspace_dir
                .join("instances")
                .join(&self.instance_slug)
                .join("agents")
                .join("observer.toml");
            if let Ok(raw) = std::fs::read_to_string(&observer_path) {
                if let Ok(mut agent) =
                    toml::from_str::<crate::domain::child_agent::ChildAgentConfig>(&raw)
                {
                    agent.enabled = enabled;
                    if let Ok(toml_str) = toml::to_string_pretty(&agent) {
                        let _ = std::fs::write(&observer_path, toml_str);
                        changes.push(format!(
                            "observer agent → {}",
                            if enabled { "enabled" } else { "disabled" }
                        ));
                    }
                }
            }

            // Immediately start or stop recording on connected machines
            let registry = self.machine_registry.clone();
            tokio::spawn(async move {
                let machines = registry.list().await;
                for m in &machines {
                    if !m.screen_recording_allowed {
                        continue;
                    }
                    if enabled {
                        crate::services::tools::screen::start_recording_on_machine(
                            &registry,
                            &m.machine_id,
                            &m.os,
                        )
                        .await;
                    } else {
                        let stop = crate::services::machine_registry::AgentToolCall {
                            request_id: uuid::Uuid::new_v4().to_string(),
                            action: "bash".into(),
                            params: serde_json::json!({
                                "command": "pkill -f 'ffmpeg.*nolune_screen' 2>/dev/null; echo stopped"
                            }),
                        };
                        let _ = registry.execute(&m.machine_id, stop).await;
                    }
                }
            });
        }

        // --- Agent interval ---
        if let Some(ref ai) = args.agent_interval {
            let agent_path = self
                .workspace_dir
                .join("instances")
                .join(&self.instance_slug)
                .join("agents")
                .join(format!("{}.toml", ai.agent_name));
            if !agent_path.exists() {
                return Err(ToolExecError(format!(
                    "agent '{}' not found",
                    ai.agent_name
                )));
            }
            let raw = std::fs::read_to_string(&agent_path)
                .map_err(|e| ToolExecError(format!("failed to read agent config: {e}")))?;
            let mut agent: crate::domain::child_agent::ChildAgentConfig = toml::from_str(&raw)
                .map_err(|e| ToolExecError(format!("failed to parse agent config: {e}")))?;
            agent.interval_hours = ai.interval_hours;
            let toml_str = toml::to_string_pretty(&agent)
                .map_err(|e| ToolExecError(format!("failed to serialize: {e}")))?;
            std::fs::write(&agent_path, toml_str)
                .map_err(|e| ToolExecError(format!("failed to write agent config: {e}")))?;
            changes.push(format!(
                "agent '{}' interval → {}h (takes effect on restart)",
                ai.agent_name, ai.interval_hours
            ));
        }

        // --- Reset agent to defaults ---
        if let Some(ref name) = args.reset_agent {
            match crate::services::child_agents::get_builtin_default(name) {
                Some(default_config) => {
                    let agent_path = self
                        .workspace_dir
                        .join("instances")
                        .join(&self.instance_slug)
                        .join("agents")
                        .join(format!("{name}.toml"));
                    let toml_str = toml::to_string_pretty(&default_config)
                        .map_err(|e| ToolExecError(format!("failed to serialize: {e}")))?;
                    std::fs::write(&agent_path, toml_str)
                        .map_err(|e| ToolExecError(format!("failed to write: {e}")))?;
                    changes.push(format!("agent '{name}' reset to defaults"));
                }
                None => {
                    return Err(ToolExecError(format!(
                        "'{name}' is not a built-in agent. built-ins: companion, reflection, night-maintenance, observer, explore-code, deep-research"
                    )));
                }
            }
        }

        // --- Email account management ---
        if let Some(acct) = &args.add_email_account {
            let mut accounts =
                crate::config::EmailAccounts::load(&self.workspace_dir, &self.instance_slug);
            let email_cfg = crate::config::EmailConfig {
                smtp_host: acct.smtp_host.clone(),
                smtp_port: acct.smtp_port,
                smtp_user: acct.smtp_user.clone(),
                smtp_password: acct.smtp_password.clone(),
                smtp_from: acct.smtp_from.clone(),
                imap_host: acct.imap_host.clone(),
                imap_port: acct.imap_port,
                imap_user: acct.imap_user.clone(),
                imap_password: acct.imap_password.clone(),
            };
            accounts.push(email_cfg);
            crate::config::EmailAccounts::save(&accounts, &self.workspace_dir, &self.instance_slug)
                .map_err(|e| ToolExecError(format!("failed to save email account: {e}")))?;
            changes.push(format!("added email account {}", acct.smtp_from));
        }

        if let Some(email) = &args.remove_email_account {
            let email = email.trim().to_string();
            let mut accounts =
                crate::config::EmailAccounts::load(&self.workspace_dir, &self.instance_slug);
            let before = accounts.len();
            accounts
                .retain(|a| a.smtp_from != email && a.smtp_user != email && a.imap_user != email);
            if accounts.len() == before {
                return Err(ToolExecError(format!("email account '{email}' not found")));
            }
            crate::config::EmailAccounts::save(&accounts, &self.workspace_dir, &self.instance_slug)
                .map_err(|e| ToolExecError(format!("failed to save email config: {e}")))?;
            changes.push(format!("removed email account {email}"));
        }

        if changes.is_empty() {
            return Ok("nothing to change — all fields were null".into());
        }

        // Save global config if anything changed there
        if args.model_mode.is_some()
            || args.openai_key.is_some()
            || args.anthropic_key.is_some()
            || args.brave_search_key.is_some()
            || args.add_mcp_server.is_some()
            || args.remove_mcp_server.is_some()
        {
            let output = toml::to_string_pretty(&config)
                .map_err(|e| ToolExecError(format!("failed to serialize config: {e}")))?;
            fs::write(&self.config_path, &output)
                .map_err(|e| ToolExecError(format!("failed to write config: {e}")))?;
        }

        Ok(format!(
            "updated: {}. changes take effect on next message.",
            changes.join(", ")
        ))
    }
}

// ---------------------------------------------------------------------------
// clear_context
// ---------------------------------------------------------------------------

pub struct ClearContextTool {
    workspace_dir: PathBuf,
    instance_slug: String,
    chat_id: String,
    events: broadcast::Sender<ServerEvent>,
}

impl ClearContextTool {
    pub fn new(
        workspace_dir: &Path,
        instance_slug: &str,
        chat_id: &str,
        events: broadcast::Sender<ServerEvent>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
            chat_id: chat_id.to_string(),
            events,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ClearContextArgs {
    /// If true, also clears chat message history. Default: false (only clears compacted summary).
    #[serde(default)]
    pub clear_messages: bool,
}

impl Tool for ClearContextTool {
    const NAME: &'static str = "clear_context";
    type Error = ToolExecError;
    type Args = ClearContextArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "clear_context".into(),
            description:
                "Clear compacted context. Set clear_messages=true to also wipe chat history.".into(),
            parameters: openai_schema::<ClearContextArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if args.clear_messages {
            // Full clear — use the same service function as the HTTP endpoint
            chat::clear_context(&self.workspace_dir, &self.instance_slug, &self.chat_id);
        } else {
            // Only clear compacted summary
            let compact =
                chat::compact_path(&self.workspace_dir, &self.instance_slug, &self.chat_id);
            if compact.exists() {
                let _ = fs::remove_file(&compact);
            }
        }

        // Clear temporary voice override
        super::companion::clear_voice_override(&self.instance_slug);

        // Broadcast snapshot so the client UI updates immediately
        if args.clear_messages {
            let _ = self.events.send(ServerEvent::ChatSnapshot {
                instance_slug: self.instance_slug.clone(),
                chat_id: self.chat_id.clone(),
                messages: vec![],
                agent_running: true,
            });
        }

        let what = if args.clear_messages {
            "compacted context and chat history cleared"
        } else {
            "compacted context cleared — chat history preserved"
        };
        Ok(what.to_string())
    }
}

// ---------------------------------------------------------------------------
// create_drop
// ---------------------------------------------------------------------------

pub struct CreateDropTool {
    workspace_dir: PathBuf,
    instance_slug: String,
    events: broadcast::Sender<ServerEvent>,
}

impl CreateDropTool {
    pub fn new(
        workspace_dir: &Path,
        instance_slug: &str,
        events: broadcast::Sender<ServerEvent>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
            events,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateDropArgs {
    /// The kind of drop: thought, idea, poem, observation, reflection, recommendation, story, question, or note.
    pub kind: String,
    /// A short title for this drop (a few words).
    pub title: String,
    /// The creative content (text/markdown).
    pub content: String,
    /// Optional image URL (e.g. from fal.ai generation) to attach to the drop.
    #[serde(default)]
    pub image_url: Option<String>,
}

impl Tool for CreateDropTool {
    const NAME: &'static str = "create_drop";
    type Error = ToolExecError;
    type Args = CreateDropArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "create_drop".into(),
            description: "Create a creative drop (poem, idea, reflection, etc.). Max 3/day.".into(),
            parameters: openai_schema::<CreateDropArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let instance_dir = self
            .workspace_dir
            .join("instances")
            .join(&self.instance_slug);
        let mood = load_mood_state(&instance_dir);

        let drop = crate::services::drops::create_drop_with_image(
            &self.workspace_dir,
            &self.instance_slug,
            &args.kind,
            &args.title,
            &args.content,
            &mood.companion_mood,
            args.image_url.as_deref(),
        )
        .map_err(|e| ToolExecError(format!("failed to create drop: {e}")))?;

        let _ = self.events.send(ServerEvent::DropCreated {
            instance_slug: self.instance_slug.clone(),
            drop: drop.clone(),
        });

        Ok(format!(
            "drop created: {} ({})",
            drop.title,
            drop.kind.as_str()
        ))
    }
}

// ---------------------------------------------------------------------------
// explore_code + search_code
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct SearchCodeTool {
    instance_dir: PathBuf,
}

impl SearchCodeTool {
    #[allow(dead_code)]
    pub fn new(workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            instance_dir: workspace_dir.join("instances").join(instance_slug),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
pub struct SearchCodeArgs {
    /// Text or pattern to search for (case-insensitive substring match).
    pub query: String,
    /// Directory to search in. Absolute path (e.g. "/Users/timur/projects/app") or relative to instance root. Default: instance directory.
    pub path: Option<String>,
}

impl Tool for SearchCodeTool {
    const NAME: &'static str = "search_code";
    type Error = ToolExecError;
    type Args = SearchCodeArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "search_code".into(),
            description: "Search files for a text pattern. Returns matching lines with paths and line numbers.".into(),
            parameters: openai_schema::<SearchCodeArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let query = args.query.trim().to_lowercase();
        if query.is_empty() {
            return Err(ToolExecError("query cannot be empty".into()));
        }

        let search_dir = if let Some(ref p) = args.path {
            if p.starts_with('/') {
                PathBuf::from(p)
            } else {
                self.instance_dir.join(p)
            }
        } else {
            self.instance_dir.clone()
        };

        if !search_dir.exists() {
            return Err(ToolExecError(format!(
                "path does not exist: {}",
                search_dir.display()
            )));
        }

        let mut results = Vec::new();
        search_files_recursive(&search_dir, &query, &search_dir, &mut results, 0);

        if results.is_empty() {
            return Ok(format!("no matches for '{}'", args.query));
        }

        let truncated = results.len() > 50;
        let output: String = results
            .iter()
            .take(50)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        if truncated {
            Ok(format!(
                "{output}\n... ({} total matches, showing first 50)",
                results.len()
            ))
        } else {
            Ok(output)
        }
    }
}

#[allow(dead_code)]
fn search_files_recursive(
    dir: &Path,
    query: &str,
    base: &Path,
    results: &mut Vec<String>,
    depth: usize,
) {
    if depth > 10 || results.len() > 200 {
        return;
    }

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                name,
                "node_modules"
                    | ".git"
                    | "target"
                    | ".next"
                    | "dist"
                    | "build"
                    | ".svelte-kit"
                    | "__pycache__"
                    | ".venv"
                    | "venv"
            ) {
                continue;
            }
            search_files_recursive(&path, query, base, results, depth + 1);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(
                ext,
                "json"
                    | "md"
                    | "txt"
                    | "toml"
                    | "yaml"
                    | "yml"
                    | "rs"
                    | "ts"
                    | "js"
                    | "svelte"
                    | "css"
                    | "html"
                    | "py"
                    | "sh"
                    | ""
            ) {
                if let Ok(content) = fs::read_to_string(&path) {
                    let rel = path.strip_prefix(base).unwrap_or(&path);
                    for (i, line) in content.lines().enumerate() {
                        if line.to_lowercase().contains(query) {
                            results.push(format!("{}:{}: {}", rel.display(), i + 1, line.trim()));
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// call_agent — universal on-demand agent invocation
// ---------------------------------------------------------------------------

pub struct CallAgentTool {
    workspace_dir: PathBuf,
    instance_slug: String,
    llm: crate::services::llm::LlmBackend,
    events: tokio::sync::broadcast::Sender<crate::domain::events::ServerEvent>,
    vector_store: std::sync::Arc<crate::services::vector::VectorStore>,
    google_ai_key: String,
}

impl CallAgentTool {
    pub fn new(
        workspace_dir: &Path,
        instance_slug: &str,
        llm: crate::services::llm::LlmBackend,
        events: tokio::sync::broadcast::Sender<crate::domain::events::ServerEvent>,
        vector_store: std::sync::Arc<crate::services::vector::VectorStore>,
        google_ai_key: &str,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
            llm,
            events,
            vector_store,
            google_ai_key: google_ai_key.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct CallAgentArgs {
    /// Name of the agent to call. Built-in agents: "explore-code", "deep-research".
    /// Can also call any custom child agent by name.
    pub agent_name: String,
    /// The task or question for the agent. Be specific.
    pub task: String,
    /// Optional: delay in seconds before running. If set (> 0), the agent is scheduled
    /// for later instead of running immediately (e.g. 3600 = 1 hour, 86400 = 1 day).
    pub delay_seconds: Option<u32>,
}

impl Tool for CallAgentTool {
    const NAME: &'static str = "call_agent";
    type Error = ToolExecError;
    type Args = CallAgentArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "call_agent".into(),
            description: "Call a child agent to perform a task. Built-in agents: \
                'explore-code' (codebase exploration), 'deep-research' (web + memory research). \
                Can also call any custom agent by name. Returns the agent's summary. \
                Set delay_seconds to schedule the agent for later instead of running now \
                (e.g. 3600 = 1h, 86400 = 1d). \
                IMPORTANT: call this tool ONCE per task — do NOT call the same agent \
                repeatedly for the same question. The agent does thorough work internally."
                .into(),
            parameters: openai_schema::<CallAgentArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let task = args.task.trim();
        if task.is_empty() {
            return Err(ToolExecError("task cannot be empty".into()));
        }

        // Scheduled mode: write a timer file and return immediately
        if let Some(delay) = args.delay_seconds {
            if delay > 0 {
                let instance_dir = self
                    .workspace_dir
                    .join("instances")
                    .join(&self.instance_slug);
                let now = chrono::Utc::now().timestamp();
                let scheduled = crate::services::tools::communication::ScheduledTask {
                    id: uuid::Uuid::new_v4().to_string(),
                    task: task.to_string(),
                    deliver_at: now + delay as i64,
                    created_at: now,
                };
                let schedule_dir = instance_dir.join("scheduled");
                std::fs::create_dir_all(&schedule_dir).map_err(|e| ToolExecError(e.to_string()))?;
                let file_path = schedule_dir.join(format!("{}.json", scheduled.id));
                let json = serde_json::to_string_pretty(&scheduled)
                    .map_err(|e| ToolExecError(e.to_string()))?;
                std::fs::write(&file_path, json).map_err(|e| ToolExecError(e.to_string()))?;

                let time_desc = if delay >= 86400 {
                    let d = delay / 86400;
                    let h = (delay % 86400) / 3600;
                    if h > 0 {
                        format!("{d}d {h}h")
                    } else {
                        format!("{d}d")
                    }
                } else if delay >= 3600 {
                    let h = delay / 3600;
                    let m = (delay % 3600) / 60;
                    if m > 0 {
                        format!("{h}h {m}m")
                    } else {
                        format!("{h}h")
                    }
                } else if delay >= 60 {
                    let m = delay / 60;
                    let s = delay % 60;
                    if s > 0 {
                        format!("{m}m {s}s")
                    } else {
                        format!("{m}m")
                    }
                } else {
                    format!("{delay}s")
                };
                return Ok(format!("agent wake-up scheduled in {time_desc}."));
            }
        }

        // Immediate mode: run the agent now
        let agents =
            crate::services::child_agents::load_agents(&self.workspace_dir, &self.instance_slug);
        let agent = agents
            .iter()
            .find(|a| a.name == args.agent_name)
            .ok_or_else(|| {
                let available: Vec<_> = agents.iter().map(|a| a.name.as_str()).collect();
                ToolExecError(format!(
                    "agent '{}' not found. available: {}",
                    args.agent_name,
                    available.join(", ")
                ))
            })?;

        if !agent.enabled {
            return Err(ToolExecError(format!(
                "agent '{}' is disabled",
                args.agent_name
            )));
        }

        let instance_dir = self
            .workspace_dir
            .join("instances")
            .join(&self.instance_slug);

        log::info!("[call_agent] invoking '{}' with task: {task}", agent.name);

        let r = crate::services::child_agents::run_single_agent(
            &self.workspace_dir,
            &self.instance_slug,
            &instance_dir,
            &self.llm,
            &self.events,
            &self.vector_store,
            &self.google_ai_key,
            agent,
            Some(task),
            &format!("tool:call_agent:{}", agent.name),
            None,
        )
        .await
        .map_err(|e| ToolExecError(format!("agent '{}' failed: {e}", agent.name)))?;

        log::info!(
            "[call_agent] '{}' completed ({} tokens, {})",
            agent.name,
            r.tokens,
            r.run_id
        );
        Ok(format!("{}\n\n[agent run: {}]", r.response, r.run_id))
    }
}

// ---------------------------------------------------------------------------
// request_secret
// ---------------------------------------------------------------------------

/// Allowed target paths for secrets. Extensible for future MCP connectors.
/// Write a secret value to a file.
/// - For `.toml` files with a dotted `key`: sets the value at that path (e.g. "llm.tokens.anthropic").
/// - For `.json` files with a dotted `key`: sets the value at that path.
/// - Otherwise (or no key): writes the raw value as the entire file content.
fn write_secret_to_file(
    file_path: &Path,
    key: Option<&str>,
    value: &str,
) -> Result<(), ToolExecError> {
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| ToolExecError(format!("failed to create directory: {e}")))?;
    }

    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    match (ext, key) {
        ("toml", Some(key)) => {
            let raw = fs::read_to_string(file_path).unwrap_or_default();
            let mut root: toml::Value = if raw.trim().is_empty() {
                toml::Value::Table(toml::map::Map::new())
            } else {
                toml::from_str(&raw)
                    .map_err(|e| ToolExecError(format!("failed to parse TOML: {e}")))?
            };

            let parts: Vec<&str> = key.split('.').collect();
            let mut current = &mut root;
            for &k in &parts[..parts.len() - 1] {
                let table = current
                    .as_table_mut()
                    .ok_or_else(|| ToolExecError(format!("path component is not a table: {k}")))?;
                if !table.contains_key(k) {
                    table.insert(k.to_string(), toml::Value::Table(toml::map::Map::new()));
                }
                current = table.get_mut(k).unwrap();
            }
            let leaf = parts.last().unwrap();
            current
                .as_table_mut()
                .ok_or_else(|| ToolExecError(format!("parent of '{leaf}' is not a table")))?
                .insert(leaf.to_string(), toml::Value::String(value.to_string()));

            let output = toml::to_string_pretty(&root)
                .map_err(|e| ToolExecError(format!("failed to serialize TOML: {e}")))?;
            fs::write(file_path, output)
                .map_err(|e| ToolExecError(format!("failed to write file: {e}")))?;
        }
        ("json", Some(key)) => {
            let raw = fs::read_to_string(file_path).unwrap_or_default();
            let mut root: serde_json::Value = if raw.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&raw)
                    .map_err(|e| ToolExecError(format!("failed to parse JSON: {e}")))?
            };

            let parts: Vec<&str> = key.split('.').collect();
            let mut current = &mut root;
            for &k in &parts[..parts.len() - 1] {
                if !current.get(k).is_some_and(|v| v.is_object()) {
                    current[k] = serde_json::json!({});
                }
                current = current.get_mut(k).unwrap();
            }
            let leaf = parts.last().unwrap();
            current[*leaf] = serde_json::Value::String(value.to_string());

            let output = serde_json::to_string_pretty(&root)
                .map_err(|e| ToolExecError(format!("failed to serialize JSON: {e}")))?;
            fs::write(file_path, output)
                .map_err(|e| ToolExecError(format!("failed to write file: {e}")))?;
        }
        _ => {
            // Plain file — write raw value (e.g. .env, .key, .txt)
            fs::write(file_path, value)
                .map_err(|e| ToolExecError(format!("failed to write file: {e}")))?;
        }
    }

    Ok(())
}

pub struct RequestSecretTool {
    instance_slug: String,
    workspace_dir: PathBuf,
    events: broadcast::Sender<ServerEvent>,
    pending_secrets: Arc<tokio::sync::Mutex<HashMap<String, PendingSecret>>>,
}

impl RequestSecretTool {
    pub fn new(
        workspace_dir: &Path,
        instance_slug: &str,
        _config_path: &Path,
        events: broadcast::Sender<ServerEvent>,
        pending_secrets: Arc<tokio::sync::Mutex<HashMap<String, PendingSecret>>>,
    ) -> Self {
        Self {
            instance_slug: instance_slug.to_string(),
            workspace_dir: workspace_dir.to_path_buf(),
            events,
            pending_secrets,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct RequestSecretArgs {
    /// A user-facing prompt explaining what secret is needed (e.g. "Enter your API key").
    pub prompt: String,
    /// File path where the secret will be stored. Can be absolute or relative to the workspace.
    /// For .toml/.json files, use `key` to set a specific field. For other files, the value is written as-is.
    pub file: String,
    /// Optional dotted key path for .toml/.json files (e.g. "llm.tokens.anthropic", "password").
    /// If omitted, the entire file content is replaced with the secret value.
    #[serde(default)]
    pub key: Option<String>,
}

impl Tool for RequestSecretTool {
    const NAME: &'static str = "request_secret";
    type Error = ToolExecError;
    type Args = RequestSecretArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "request_secret".into(),
            description:
                "Prompt user for a secret (API key, password, token) via secure masked input. \
                Written directly to the specified file, never visible to you. \
                For github token: use file=\"config\", key=\"github.token\"."
                    .into(),
            parameters: openai_schema::<RequestSecretArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let file_path = if args.file.starts_with('/') {
            PathBuf::from(&args.file)
        } else {
            self.workspace_dir.join(&args.file)
        };
        let target = if let Some(ref key) = args.key {
            format!("{}:{}", args.file, key)
        } else {
            args.file.clone()
        };

        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();

        {
            let mut secrets = self.pending_secrets.lock().await;
            secrets.insert(
                id.clone(),
                PendingSecret {
                    target: target.clone(),
                    responder: tx,
                },
            );
        }

        // Broadcast the request to the client
        let _ = self.events.send(ServerEvent::SecretRequest {
            instance_slug: self.instance_slug.clone(),
            id: id.clone(),
            prompt: args.prompt.clone(),
            target: target.clone(),
        });

        log::info!(
            "[request_secret] waiting for user input (id={}, target={})",
            &id[..8],
            target
        );

        // Wait for the user's response with a 5-minute timeout
        let value = tokio::time::timeout(std::time::Duration::from_secs(300), rx)
            .await
            .map_err(|_| {
                // Clean up on timeout
                let pending = self.pending_secrets.clone();
                let id_clone = id.clone();
                tokio::spawn(async move {
                    pending.lock().await.remove(&id_clone);
                });
                ToolExecError("secret request timed out after 5 minutes".into())
            })?
            .map_err(|_| ToolExecError("secret request was cancelled".into()))?;

        // Special handling for well-known config targets
        if args.file == "config" || args.file == "instance_config" {
            match args.key.as_deref() {
                Some("github.token" | "github_token") => {
                    let mut instance_cfg = crate::config::InstanceConfig::load(
                        &self.workspace_dir,
                        &self.instance_slug,
                    );
                    instance_cfg.github.token = value.trim().to_string();
                    instance_cfg
                        .save(&self.workspace_dir, &self.instance_slug)
                        .map_err(|e| {
                            ToolExecError(format!("failed to save instance config: {e}"))
                        })?;
                    log::info!("[request_secret] github token saved to instance config");
                    return Ok("github token updated. changes take effect on next message.".into());
                }
                _ => {
                    return Err(ToolExecError(format!(
                        "unknown config key: {:?}. supported: github.token",
                        args.key
                    )));
                }
            }
        }

        // Write secret to file
        write_secret_to_file(&file_path, args.key.as_deref(), &value)?;

        log::info!("[request_secret] secret saved to {target}");
        Ok(format!("secret saved to {target}"))
    }
}

// ---------------------------------------------------------------------------
// restart_machine — restart through a service manager when available
// ---------------------------------------------------------------------------

pub struct RestartMachineTool;

#[derive(Deserialize, JsonSchema)]
pub struct RestartMachineArgs {
    /// Reason for the restart (logged for audit).
    pub reason: String,
}

impl Tool for RestartMachineTool {
    const NAME: &'static str = "restart_machine";
    type Error = ToolExecError;
    type Args = RestartMachineArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "restart_machine".into(),
            description: "Restart the server process through its service manager. \
                Use when the environment is broken, MCP servers are stuck, \
                or after an update that needs a clean restart."
                .into(),
            parameters: openai_schema::<RestartMachineArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        log::info!("[restart] reason: {}", args.reason);

        // Try Fly.io internal API first (if running on Fly)
        if let (Ok(app), Ok(machine_id)) = (
            std::env::var("FLY_APP_NAME"),
            std::env::var("FLY_MACHINE_ID"),
        ) {
            log::info!("[restart] Fly.io: {app}/{machine_id}");
            let client = reqwest::Client::new();
            let url =
                format!("http://_api.internal:4280/v1/apps/{app}/machines/{machine_id}/restart");
            if let Ok(resp) = client.post(&url).send().await {
                if resp.status().is_success() {
                    return Ok("restart initiated via Fly.io — back in ~10 seconds".into());
                }
            }
        }

        // Fallback: exit process, rely on supervisor to restart
        // (for example, systemd Restart=always)
        log::info!("[restart] exiting process (supervisor will restart)");
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            std::process::exit(0);
        });

        Ok("server shutting down — supervisor will restart it in a few seconds".into())
    }
}

// ---------------------------------------------------------------------------
// export_profile — create a tar.gz of the instance for the user to download
// ---------------------------------------------------------------------------

pub struct ExportProfileTool {
    workspace_dir: PathBuf,
    instance_slug: String,
    events: broadcast::Sender<ServerEvent>,
}

impl ExportProfileTool {
    pub fn new(
        workspace_dir: &Path,
        instance_slug: &str,
        events: broadcast::Sender<ServerEvent>,
    ) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
            events,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ExportProfileArgs {
    /// Reason for export (e.g. "backup", "migrating to new instance").
    #[serde(default)]
    pub _reason: Option<String>,
}

impl Tool for ExportProfileTool {
    const NAME: &'static str = "create_backup";
    type Error = ToolExecError;
    type Args = ExportProfileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "create_backup".into(),
            description: "Create a downloadable .tar.gz backup of this instance. \
                Includes soul, memory, drops, chat history, and all data. \
                Returns a download link the user can click."
                .into(),
            parameters: openai_schema::<ExportProfileArgs>(),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let instance_dir = self
            .workspace_dir
            .join("instances")
            .join(&self.instance_slug);
        if !instance_dir.is_dir() {
            return Err(ToolExecError("instance directory not found".into()));
        }

        // Create tar.gz
        let output = tokio::process::Command::new("tar")
            .arg("czf")
            .arg("-")
            .arg("-C")
            .arg(self.workspace_dir.join("instances"))
            .arg(&self.instance_slug)
            .output()
            .await
            .map_err(|e| ToolExecError(format!("failed to create archive: {e}")))?;

        if !output.status.success() {
            return Err(ToolExecError("tar failed".into()));
        }

        // Save to uploads so user can download
        let filename = format!("{}.tar.gz", self.instance_slug);
        let meta = crate::services::uploads::save_upload(
            &self.workspace_dir,
            &self.instance_slug,
            &filename,
            &output.stdout,
        )
        .map_err(|e| ToolExecError(format!("failed to save archive: {e}")))?;

        let marker = format!("[attached: {} ({})]", filename, meta.id);
        let _ = self.events.send(ServerEvent::ChatStreamDelta {
            instance_slug: self.instance_slug.clone(),
            chat_id: "default".to_string(),
            message_id: String::new(),
            delta: String::new(),
        });

        Ok(format!(
            "exported profile as {filename} ({} bytes). {marker}",
            output.stdout.len()
        ))
    }
}

// ---------------------------------------------------------------------------
// import_profile — import a tar.gz into this instance
// ---------------------------------------------------------------------------

pub struct ImportProfileTool {
    workspace_dir: PathBuf,
    instance_slug: String,
}

impl ImportProfileTool {
    pub fn new(workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            instance_slug: instance_slug.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ImportProfileArgs {
    /// Path to the .tar.gz file OR an upload ID (e.g. "upload_12345") from a user attachment.
    pub source: String,
}

impl Tool for ImportProfileTool {
    const NAME: &'static str = "restore_backup";
    type Error = ToolExecError;
    type Args = ImportProfileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "restore_backup".into(),
            description: "Restore from a .tar.gz backup archive. \
                Merges data (soul, memory, drops, chat history) from the archive. \
                Accepts a file path or an upload ID from a user attachment (e.g. 'upload_12345')."
                .into(),
            parameters: openai_schema::<ImportProfileArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let instance_dir = self
            .workspace_dir
            .join("instances")
            .join(&self.instance_slug);
        let source = args.source.trim();

        // Resolve source: upload ID or file path
        let archive_path = if source.starts_with("upload_") {
            // Look up the upload by ID
            let uploads_dir = instance_dir.join("uploads");
            let meta_path = uploads_dir.join(format!("{source}.json"));
            let meta_raw = fs::read_to_string(&meta_path)
                .map_err(|_| ToolExecError(format!("upload '{source}' not found")))?;
            let meta: serde_json::Value = serde_json::from_str(&meta_raw)
                .map_err(|_| ToolExecError("invalid upload metadata".into()))?;
            let stored_name = meta["stored_name"]
                .as_str()
                .ok_or_else(|| ToolExecError("upload has no stored_name".into()))?;
            uploads_dir.join(stored_name)
        } else if source.starts_with('/') {
            std::path::PathBuf::from(source)
        } else {
            instance_dir.join(source)
        };

        if !archive_path.is_file() {
            return Err(ToolExecError(format!(
                "file not found: {}",
                archive_path.display()
            )));
        }

        // Auto-detect: gzip magic bytes 1f 8b
        let is_gzip = fs::read(&archive_path)
            .map(|d| d.len() >= 2 && d[0] == 0x1f && d[1] == 0x8b)
            .unwrap_or(false);
        let output = tokio::process::Command::new("tar")
            .arg(if is_gzip { "xzf" } else { "xf" })
            .arg(&archive_path)
            .arg("--strip-components=1")
            .arg("-C")
            .arg(&instance_dir)
            .output()
            .await
            .map_err(|e| ToolExecError(format!("failed to extract archive: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ToolExecError(format!("tar extract failed: {stderr}")));
        }

        // Rebuild memory catalog after import
        crate::services::memory::rebuild_catalog_snapshot(&self.workspace_dir, &self.instance_slug);
        crate::services::memory::invalidate_frozen_catalog(&self.instance_slug);

        Ok(format!(
            "imported profile from {}. memory catalog rebuilt.",
            args.source
        ))
    }
}
