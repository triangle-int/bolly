use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::config::EmailConfig;
use crate::services::tool::{Tool, ToolDefinition};
use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::companion::{load_mood_state, save_mood_state};
use super::{ToolExecError, openai_schema};
use crate::domain::events::ServerEvent;
use crate::services::google::GoogleClient;

// ---------------------------------------------------------------------------
// schedule_agent
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct ScheduleAgentTool {
    instance_dir: PathBuf,
}

impl ScheduleAgentTool {
    #[allow(dead_code)]
    pub fn new(workspace_dir: &Path, instance_slug: &str) -> Self {
        Self {
            instance_dir: workspace_dir.join("instances").join(instance_slug),
        }
    }
}

/// A scheduled agent task entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub task: String,
    pub deliver_at: i64,
    pub created_at: i64,
}

/// Arguments for schedule_agent tool.
#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
pub struct ScheduleAgentArgs {
    /// What to do when the timer fires. Describe the task clearly — you'll be woken up with full tools to execute it.
    pub task: String,
    /// When to trigger, in seconds from now (e.g. 60 for "in 1 minute", 3600 for "in 1 hour", 86400 for "tomorrow").
    pub delay_seconds: u32,
}

impl Tool for ScheduleAgentTool {
    const NAME: &'static str = "schedule_agent";
    type Error = ToolExecError;
    type Args = ScheduleAgentArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "schedule_agent".into(),
            description: "Schedule yourself to wake up later and perform a task. You'll be triggered with full tools. Delay in seconds (60=1m, 3600=1h, 86400=1d).".into(),
            parameters: openai_schema::<ScheduleAgentArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let task = args.task.trim().to_string();
        if task.is_empty() {
            return Err(ToolExecError("task cannot be empty".into()));
        }

        if args.delay_seconds == 0 {
            return Err(ToolExecError("delay must be at least 1 second".into()));
        }

        let now = Utc::now().timestamp();
        let deliver_at = now + args.delay_seconds as i64;

        let scheduled = ScheduledTask {
            id: uuid::Uuid::new_v4().to_string(),
            task,
            deliver_at,
            created_at: now,
        };

        let schedule_dir = self.instance_dir.join("scheduled");
        fs::create_dir_all(&schedule_dir).map_err(|e| ToolExecError(e.to_string()))?;

        let file_path = schedule_dir.join(format!("{}.json", scheduled.id));
        let json =
            serde_json::to_string_pretty(&scheduled).map_err(|e| ToolExecError(e.to_string()))?;
        fs::write(&file_path, json).map_err(|e| ToolExecError(e.to_string()))?;

        let total = args.delay_seconds;
        let time_desc = if total >= 86400 {
            let d = total / 86400;
            let h = (total % 86400) / 3600;
            if h > 0 {
                format!("{d}d {h}h")
            } else {
                format!("{d}d")
            }
        } else if total >= 3600 {
            let h = total / 3600;
            let m = (total % 3600) / 60;
            if m > 0 {
                format!("{h}h {m}m")
            } else {
                format!("{h}h")
            }
        } else if total >= 60 {
            let m = total / 60;
            let s = total % 60;
            if s > 0 {
                format!("{m}m {s}s")
            } else {
                format!("{m}m")
            }
        } else {
            format!("{total}s")
        };

        Ok(format!("agent wake-up scheduled in {time_desc}."))
    }
}

// ---------------------------------------------------------------------------
// reach_out
// ---------------------------------------------------------------------------

pub struct ReachOutTool {
    workspace_dir: PathBuf,
    instance_slug: String,
    events: broadcast::Sender<ServerEvent>,
}

impl ReachOutTool {
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
pub struct ReachOutArgs {
    /// The message to send to the user. Keep it natural and concise.
    pub message: String,
    /// Optional image URL (e.g. from fal.ai generation) to include in the message.
    #[serde(default)]
    pub image_url: Option<String>,
}

impl Tool for ReachOutTool {
    const NAME: &'static str = "reach_out";
    type Error = ToolExecError;
    type Args = ReachOutArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "reach_out".into(),
            description: "Send a spontaneous message to the user. Use sparingly.".into(),
            parameters: openai_schema::<ReachOutArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let mut message = args.message.trim().to_string();
        if message.is_empty() {
            return Err(ToolExecError("message cannot be empty".into()));
        }
        // Append image as markdown if provided
        if let Some(url) = &args.image_url {
            let url = url.trim();
            if !url.is_empty() {
                message.push_str(&format!("\n\n![image]({url})"));
            }
        }

        let instance_dir = self
            .workspace_dir
            .join("instances")
            .join(&self.instance_slug);
        let mood = load_mood_state(&instance_dir);
        let now_ts = chrono::Utc::now().timestamp();

        let mut mood = mood;
        mood.last_reach_out = now_ts;
        save_mood_state(&instance_dir, &mood);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();

        let chat_message = crate::domain::chat::ChatMessage {
            id: format!("hb_{now}"),
            role: crate::domain::chat::ChatRole::Assistant,
            content: message.clone(),
            created_at: now.to_string(),
            kind: Default::default(),
            tool_name: None,
            mcp_app_html: None,
            mcp_app_input: None,
            model: None,
        };

        let chat_dir = self
            .workspace_dir
            .join("instances")
            .join(&self.instance_slug)
            .join("chats")
            .join("default");
        let _ = std::fs::create_dir_all(&chat_dir);
        let messages_path = chat_dir.join("messages.json");

        let lock = super::chat_file_lock(&messages_path);
        let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());

        let mut messages: Vec<crate::domain::chat::ChatMessage> =
            std::fs::read_to_string(&messages_path)
                .ok()
                .and_then(|raw| serde_json::from_str(&raw).ok())
                .unwrap_or_default();

        messages.push(chat_message.clone());

        if let Ok(json) = serde_json::to_string_pretty(&messages) {
            let _ = std::fs::write(&messages_path, json);
        }

        // Also save to rig_history (single source of truth for chat reload)
        let rig_entry = crate::services::llm::HistoryEntry::new(
            crate::services::llm::Message::assistant(&message),
            now.to_string(),
            format!("hb_{now}"),
        );
        let rig_path = chat_dir.join("rig_history.json");
        crate::services::chat::append_to_rig_history(&rig_path, &rig_entry);

        let _ = self.events.send(ServerEvent::ChatMessageCreated {
            instance_slug: self.instance_slug.clone(),
            chat_id: "default".to_string(),
            message: chat_message,
        });

        log::info!(
            "[reach_out] {} sent message: {}",
            self.instance_slug,
            &message[..message.len().min(60)]
        );
        Ok("message delivered".to_string())
    }
}

// ---------------------------------------------------------------------------
// send_email (unified: Gmail API + SMTP)
// ---------------------------------------------------------------------------

pub struct SendEmailTool {
    google: Option<GoogleClient>,
    instance_slug: String,
    imap_accounts: Vec<EmailConfig>,
}

impl SendEmailTool {
    pub fn new(
        google: Option<GoogleClient>,
        instance_slug: &str,
        imap_accounts: Vec<EmailConfig>,
    ) -> Self {
        Self {
            google,
            instance_slug: instance_slug.to_string(),
            imap_accounts,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct SendEmailArgs {
    /// Recipient email address.
    pub to: String,
    /// Email subject line.
    pub subject: String,
    /// Email body (plain text).
    pub body: String,
    /// CC recipients (comma-separated). Optional.
    pub cc: Option<String>,
    /// BCC recipients (comma-separated). Optional.
    pub bcc: Option<String>,
    /// Email address of the account to send from. Leave empty to use default.
    pub account: Option<String>,
}

impl Tool for SendEmailTool {
    const NAME: &'static str = "send_email";
    type Error = ToolExecError;
    type Args = SendEmailArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "send_email".into(),
            description: "Send an email. Use 'account' to pick which email account to send from."
                .into(),
            parameters: openai_schema::<SendEmailArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Try to match account to an SMTP config first
        if let Some(ref acct) = args.account {
            if let Some(cfg) = self
                .imap_accounts
                .iter()
                .find(|c| c.smtp_from == *acct || c.smtp_user == *acct)
            {
                return send_via_smtp(cfg, &args).await;
            }
        }

        // Try Gmail
        if let Some(ref google) = self.google {
            if let Ok((token, email)) = google
                .access_token(&self.instance_slug, args.account.as_deref())
                .await
            {
                return send_via_gmail(&token, &email, &args).await;
            }
        }

        // No account matched — fall back to first SMTP account
        if let Some(cfg) = self.imap_accounts.first() {
            return send_via_smtp(cfg, &args).await;
        }

        Err(ToolExecError(
            "no email account available — connect Google or configure SMTP in settings".into(),
        ))
    }
}

async fn send_via_gmail(
    token: &str,
    email: &str,
    args: &SendEmailArgs,
) -> Result<String, ToolExecError> {
    let mut rfc2822 = format!(
        "From: {email}\r\nTo: {}\r\nSubject: {}\r\n",
        args.to, args.subject
    );
    if let Some(cc) = &args.cc {
        rfc2822.push_str(&format!("Cc: {cc}\r\n"));
    }
    if let Some(bcc) = &args.bcc {
        rfc2822.push_str(&format!("Bcc: {bcc}\r\n"));
    }
    rfc2822.push_str("Content-Type: text/plain; charset=utf-8\r\n\r\n");
    rfc2822.push_str(&args.body);

    use base64::Engine;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rfc2822.as_bytes());

    let client = reqwest::Client::new();
    let res = client
        .post("https://gmail.googleapis.com/gmail/v1/users/me/messages/send")
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "raw": encoded }))
        .send()
        .await
        .map_err(|e| ToolExecError(format!("Gmail API request failed: {e}")))?;

    if !res.status().is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(ToolExecError(format!("Gmail send failed: {body}")));
    }

    Ok(format!(
        "email sent to {} (from {email}, via gmail)",
        args.to
    ))
}

async fn send_via_smtp(cfg: &EmailConfig, args: &SendEmailArgs) -> Result<String, ToolExecError> {
    use lettre::{
        AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
        message::{Mailbox, MessageBuilder, header::ContentType},
        transport::smtp::authentication::Credentials,
    };

    let from: Mailbox = cfg
        .smtp_from
        .parse()
        .map_err(|e| ToolExecError(format!("invalid smtp_from address: {e}")))?;
    let to: Mailbox = args
        .to
        .parse()
        .map_err(|e| ToolExecError(format!("invalid recipient address: {e}")))?;

    let mut builder: MessageBuilder = lettre::Message::builder()
        .from(from)
        .to(to)
        .subject(&args.subject);

    if let Some(cc) = &args.cc {
        for addr in cc.split(',') {
            let addr = addr.trim();
            if !addr.is_empty() {
                let mbox: Mailbox = addr
                    .parse()
                    .map_err(|e| ToolExecError(format!("invalid CC address '{addr}': {e}")))?;
                builder = builder.cc(mbox);
            }
        }
    }
    if let Some(bcc) = &args.bcc {
        for addr in bcc.split(',') {
            let addr = addr.trim();
            if !addr.is_empty() {
                let mbox: Mailbox = addr
                    .parse()
                    .map_err(|e| ToolExecError(format!("invalid BCC address '{addr}': {e}")))?;
                builder = builder.bcc(mbox);
            }
        }
    }

    let message = builder
        .header(ContentType::TEXT_PLAIN)
        .body(args.body.clone())
        .map_err(|e| ToolExecError(format!("failed to build email: {e}")))?;

    let creds = Credentials::new(cfg.smtp_user.clone(), cfg.smtp_password.clone());

    let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.smtp_host)
        .map_err(|e| ToolExecError(format!("SMTP connection failed: {e}")))?
        .port(cfg.smtp_port)
        .credentials(creds)
        .build();

    transport
        .send(message)
        .await
        .map_err(|e| ToolExecError(format!("SMTP send failed: {e}")))?;

    Ok(format!(
        "email sent to {} (from {}, via smtp)",
        args.to, cfg.smtp_from
    ))
}

// ---------------------------------------------------------------------------
// read_email (unified: Gmail API + IMAP)
// ---------------------------------------------------------------------------

pub struct ReadEmailTool {
    google: Option<GoogleClient>,
    instance_slug: String,
    imap_accounts: Vec<EmailConfig>,
}

impl ReadEmailTool {
    pub fn new(
        google: Option<GoogleClient>,
        instance_slug: &str,
        imap_accounts: Vec<EmailConfig>,
    ) -> Self {
        Self {
            google,
            instance_slug: instance_slug.to_string(),
            imap_accounts,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ReadEmailArgs {
    /// Number of recent emails to fetch (default 5, max 20).
    #[serde(default = "default_email_count")]
    pub count: u32,
    /// Search query. For Gmail: "from:alice@example.com", "is:unread". For IMAP: "UNSEEN", "FROM \"alice\"". Optional.
    pub query: Option<String>,
    /// Mailbox/label (e.g. "INBOX", "SENT", "STARRED"). Optional, defaults to INBOX.
    pub label: Option<String>,
    /// Email address of the account to read from. Leave empty to use default.
    pub account: Option<String>,
}

fn default_email_count() -> u32 {
    5
}

impl Tool for ReadEmailTool {
    const NAME: &'static str = "read_email";
    type Error = ToolExecError;
    type Args = ReadEmailArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "read_email".into(),
            description:
                "Read recent emails. Use 'account' to pick which email account to read from.".into(),
            parameters: openai_schema::<ReadEmailArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Try to match account to an IMAP config first
        if let Some(ref acct) = args.account {
            if let Some(cfg) = self
                .imap_accounts
                .iter()
                .find(|c| c.imap_user == *acct || c.smtp_from == *acct)
            {
                return read_via_imap(cfg, &args).await;
            }
        }

        // Try Gmail
        if let Some(ref google) = self.google {
            if let Ok((token, _email)) = google
                .access_token(&self.instance_slug, args.account.as_deref())
                .await
            {
                return read_via_gmail(&token, &args).await;
            }
        }

        // No account matched — fall back to first IMAP account
        if let Some(cfg) = self.imap_accounts.first() {
            return read_via_imap(cfg, &args).await;
        }

        Err(ToolExecError(
            "no email account available — connect Google or configure IMAP in settings".into(),
        ))
    }
}

async fn read_via_gmail(token: &str, args: &ReadEmailArgs) -> Result<String, ToolExecError> {
    let count = args.count.min(20).max(1);
    let client = reqwest::Client::new();

    let mut params = vec![("maxResults".to_string(), count.to_string())];
    if let Some(q) = &args.query {
        params.push(("q".to_string(), q.clone()));
    }
    if let Some(label) = &args.label {
        params.push(("labelIds".to_string(), label.clone()));
    } else {
        params.push(("labelIds".to_string(), "INBOX".to_string()));
    }

    let list_res = client
        .get("https://gmail.googleapis.com/gmail/v1/users/me/messages")
        .header("Authorization", format!("Bearer {token}"))
        .query(&params)
        .send()
        .await
        .map_err(|e| ToolExecError(format!("Gmail list failed: {e}")))?;

    if !list_res.status().is_success() {
        let body = list_res.text().await.unwrap_or_default();
        return Err(ToolExecError(format!("Gmail list failed: {body}")));
    }

    let list_data: serde_json::Value = list_res
        .json()
        .await
        .map_err(|e| ToolExecError(format!("Gmail parse failed: {e}")))?;

    let messages = match list_data["messages"].as_array() {
        Some(m) => m,
        None => return Ok("no emails found".into()),
    };

    let mut result = String::new();
    for msg_ref in messages {
        let msg_id = match msg_ref["id"].as_str() {
            Some(id) => id,
            None => continue,
        };

        let msg_res = client
            .get(&format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{msg_id}?format=metadata&metadataHeaders=From&metadataHeaders=Subject&metadataHeaders=Date"
            ))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| ToolExecError(format!("Gmail get message failed: {e}")))?;

        if !msg_res.status().is_success() {
            continue;
        }

        let msg_data: serde_json::Value = msg_res.json().await.unwrap_or_default();

        let headers = msg_data["payload"]["headers"].as_array();
        let get_header = |name: &str| -> String {
            headers
                .and_then(|h| h.iter().find(|hdr| hdr["name"].as_str() == Some(name)))
                .and_then(|hdr| hdr["value"].as_str())
                .unwrap_or("")
                .to_string()
        };

        let from = get_header("From");
        let subject = get_header("Subject");
        let date = get_header("Date");
        let snippet = msg_data["snippet"].as_str().unwrap_or("");

        result.push_str(&format!(
            "--- email ---\nfrom: {from}\ndate: {date}\nsubject: {subject}\nsnippet: {snippet}\n\n"
        ));
    }

    if result.is_empty() {
        Ok("no emails found".into())
    } else {
        Ok(result)
    }
}

async fn read_via_imap(cfg: &EmailConfig, args: &ReadEmailArgs) -> Result<String, ToolExecError> {
    let config = cfg.clone();
    let count = args.count.min(20).max(1);
    let mailbox = args.label.clone().unwrap_or_else(|| "INBOX".to_string());
    let search = args.query.clone();

    tokio::task::spawn_blocking(move || {
        let tls = native_tls::TlsConnector::builder()
            .build()
            .map_err(|e| ToolExecError(format!("TLS error: {e}")))?;

        let client = imap::connect(
            (config.imap_host.as_str(), config.imap_port),
            &config.imap_host,
            &tls,
        ).map_err(|e| ToolExecError(format!("IMAP connect failed: {e}")))?;

        let mut session = client
            .login(&config.imap_user, &config.imap_password)
            .map_err(|(e, _)| ToolExecError(format!("IMAP login failed: {e}")))?;

        session.select(&mailbox)
            .map_err(|e| ToolExecError(format!("IMAP select '{mailbox}' failed: {e}")))?;

        let search_query = search.as_deref().unwrap_or("ALL");
        let uids = session.search(search_query)
            .map_err(|e| ToolExecError(format!("IMAP search failed: {e}")))?;

        if uids.is_empty() {
            let _ = session.logout();
            return Ok("no emails found".to_string());
        }

        let mut uid_list: Vec<u32> = uids.into_iter().collect();
        uid_list.sort();
        let start = uid_list.len().saturating_sub(count as usize);
        let recent_uids: Vec<String> = uid_list[start..]
            .iter()
            .rev()
            .map(|u| u.to_string())
            .collect();
        let uid_set = recent_uids.join(",");

        let fetches = session.fetch(&uid_set, "ENVELOPE BODY.PEEK[TEXT]<0.500>")
            .map_err(|e| ToolExecError(format!("IMAP fetch failed: {e}")))?;

        let mut result = String::new();
        for fetch in fetches.iter() {
            if let Some(envelope) = fetch.envelope() {
                let subject = envelope.subject
                    .as_ref()
                    .and_then(|s| std::str::from_utf8(s).ok())
                    .unwrap_or("(no subject)");

                let from = envelope.from.as_ref()
                    .and_then(|addrs| addrs.first())
                    .map(|a| {
                        let name = a.name.as_ref()
                            .and_then(|n| std::str::from_utf8(n).ok())
                            .unwrap_or("");
                        let mbox = a.mailbox.as_ref()
                            .and_then(|m| std::str::from_utf8(m).ok())
                            .unwrap_or("?");
                        let host = a.host.as_ref()
                            .and_then(|h| std::str::from_utf8(h).ok())
                            .unwrap_or("?");
                        if name.is_empty() {
                            format!("{mbox}@{host}")
                        } else {
                            format!("{name} <{mbox}@{host}>")
                        }
                    })
                    .unwrap_or_else(|| "(unknown)".to_string());

                let date = envelope.date.as_ref()
                    .and_then(|d| std::str::from_utf8(d).ok())
                    .unwrap_or("(no date)");

                let snippet = fetch.text()
                    .and_then(|t| std::str::from_utf8(t).ok())
                    .unwrap_or("")
                    .chars()
                    .take(300)
                    .collect::<String>()
                    .replace('\r', "")
                    .replace('\n', " ");

                result.push_str(&format!(
                    "--- email ---\nfrom: {from}\ndate: {date}\nsubject: {subject}\nsnippet: {snippet}\n\n"
                ));
            }
        }

        let _ = session.logout();

        if result.is_empty() {
            Ok("no emails found".to_string())
        } else {
            Ok(result)
        }
    })
    .await
    .map_err(|e| ToolExecError(format!("IMAP task failed: {e}")))?
}
