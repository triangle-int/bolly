use crate::services::tool::{Tool, ToolDefinition};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{ToolExecError, openai_schema};
use crate::services::google::GoogleClient;

// ---------------------------------------------------------------------------
// list_events
// ---------------------------------------------------------------------------

pub struct ListEventsTool {
    google: GoogleClient,
    instance_slug: String,
}

impl ListEventsTool {
    pub fn new(google: GoogleClient, instance_slug: &str) -> Self {
        Self {
            google,
            instance_slug: instance_slug.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ListEventsArgs {
    /// Number of days ahead to look (default 7, max 30).
    #[serde(default = "default_days")]
    pub days_ahead: u32,
    /// Free-text search query to filter events. Optional.
    pub query: Option<String>,
    /// Email address of the Google account to use. Leave empty to use default.
    pub account: Option<String>,
}

fn default_days() -> u32 {
    7
}

impl Tool for ListEventsTool {
    const NAME: &'static str = "list_events";
    type Error = ToolExecError;
    type Args = ListEventsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "list_events".into(),
            description: "List upcoming Google Calendar events for the next N days.".into(),
            parameters: openai_schema::<ListEventsArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let (token, _) = self
            .google
            .access_token(&self.instance_slug, args.account.as_deref())
            .await
            .map_err(|e| ToolExecError(e))?;

        let days = args.days_ahead.min(30).max(1);
        let now = chrono::Utc::now();
        let time_min = now.to_rfc3339();
        let time_max = (now + chrono::Duration::days(days as i64)).to_rfc3339();

        let client = reqwest::Client::new();
        let mut params = vec![
            ("timeMin", time_min.as_str()),
            ("timeMax", time_max.as_str()),
            ("singleEvents", "true"),
            ("orderBy", "startTime"),
            ("maxResults", "25"),
        ];

        let query_str;
        if let Some(q) = &args.query {
            query_str = q.clone();
            params.push(("q", &query_str));
        }

        let res = client
            .get("https://www.googleapis.com/calendar/v3/calendars/primary/events")
            .header("Authorization", format!("Bearer {token}"))
            .query(&params)
            .send()
            .await
            .map_err(|e| ToolExecError(format!("Calendar API failed: {e}")))?;

        if !res.status().is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(ToolExecError(format!("Calendar API error: {body}")));
        }

        let data: serde_json::Value = res
            .json()
            .await
            .map_err(|e| ToolExecError(format!("Calendar parse failed: {e}")))?;

        let items = match data["items"].as_array() {
            Some(items) if !items.is_empty() => items,
            _ => return Ok("no upcoming events found".into()),
        };

        let mut result = String::new();
        for event in items {
            let summary = event["summary"].as_str().unwrap_or("(untitled)");
            let start = event["start"]["dateTime"]
                .as_str()
                .or_else(|| event["start"]["date"].as_str())
                .unwrap_or("?");
            let end = event["end"]["dateTime"]
                .as_str()
                .or_else(|| event["end"]["date"].as_str())
                .unwrap_or("?");
            let location = event["location"].as_str().unwrap_or("");
            let description = event["description"].as_str().unwrap_or("");

            result.push_str(&format!(
                "--- event ---\ntitle: {summary}\nstart: {start}\nend: {end}\n"
            ));
            if !location.is_empty() {
                result.push_str(&format!("location: {location}\n"));
            }
            if !description.is_empty() {
                let preview: String = description.chars().take(200).collect();
                result.push_str(&format!("description: {preview}\n"));
            }

            if let Some(attendees) = event["attendees"].as_array() {
                let names: Vec<&str> = attendees
                    .iter()
                    .filter_map(|a| a["email"].as_str())
                    .take(10)
                    .collect();
                if !names.is_empty() {
                    result.push_str(&format!("attendees: {}\n", names.join(", ")));
                }
            }
            result.push('\n');
        }

        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// create_event
// ---------------------------------------------------------------------------

pub struct CreateEventTool {
    google: GoogleClient,
    instance_slug: String,
}

impl CreateEventTool {
    pub fn new(google: GoogleClient, instance_slug: &str) -> Self {
        Self {
            google,
            instance_slug: instance_slug.to_string(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateEventArgs {
    /// Event title/summary.
    pub summary: String,
    /// Start time in ISO 8601 format (e.g. "2025-01-15T10:00:00-05:00").
    pub start: String,
    /// End time in ISO 8601 format.
    pub end: String,
    /// Event description. Optional.
    pub description: Option<String>,
    /// Event location. Optional.
    pub location: Option<String>,
    /// Comma-separated email addresses of attendees. Optional.
    pub attendees: Option<String>,
    /// Email address of the Google account to use. Leave empty to use default.
    pub account: Option<String>,
}

impl Tool for CreateEventTool {
    const NAME: &'static str = "create_event";
    type Error = ToolExecError;
    type Args = CreateEventArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "create_event".into(),
            description: "Create a Google Calendar event. Times in ISO 8601 with timezone.".into(),
            parameters: openai_schema::<CreateEventArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let (token, _) = self
            .google
            .access_token(&self.instance_slug, args.account.as_deref())
            .await
            .map_err(|e| ToolExecError(e))?;

        let mut event = serde_json::json!({
            "summary": args.summary,
            "start": { "dateTime": args.start },
            "end": { "dateTime": args.end },
        });

        if let Some(desc) = &args.description {
            event["description"] = serde_json::Value::String(desc.clone());
        }
        if let Some(loc) = &args.location {
            event["location"] = serde_json::Value::String(loc.clone());
        }
        if let Some(attendees_str) = &args.attendees {
            let attendees: Vec<serde_json::Value> = attendees_str
                .split(',')
                .map(|e| serde_json::json!({ "email": e.trim() }))
                .collect();
            event["attendees"] = serde_json::Value::Array(attendees);
        }

        let client = reqwest::Client::new();
        let res = client
            .post("https://www.googleapis.com/calendar/v3/calendars/primary/events")
            .header("Authorization", format!("Bearer {token}"))
            .json(&event)
            .send()
            .await
            .map_err(|e| ToolExecError(format!("Calendar create failed: {e}")))?;

        if !res.status().is_success() {
            let body = res.text().await.unwrap_or_default();
            return Err(ToolExecError(format!("Calendar create error: {body}")));
        }

        let data: serde_json::Value = res.json().await.unwrap_or_default();
        let link = data["htmlLink"].as_str().unwrap_or("");
        Ok(format!("event '{}' created. link: {link}", args.summary))
    }
}
