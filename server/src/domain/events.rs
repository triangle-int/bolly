use serde::Serialize;

use crate::domain::{chat::ChatMessage, drop::Drop, thought::Thought};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    ChatMessageCreated {
        instance_slug: String,
        chat_id: String,
        message: ChatMessage,
    },
    MoodUpdated {
        instance_slug: String,
        mood: String,
    },
    AgentRunning {
        instance_slug: String,
        chat_id: String,
    },
    AgentStopped {
        instance_slug: String,
        chat_id: String,
    },
    DropCreated {
        instance_slug: String,
        drop: Drop,
    },
    HeartbeatThought {
        instance_slug: String,
        thought: Thought,
    },
    ContextCompacting {
        instance_slug: String,
        chat_id: String,
        messages_compacted: usize,
    },
    ChatStreamDelta {
        instance_slug: String,
        chat_id: String,
        message_id: String,
        delta: String,
    },
    SecretRequest {
        instance_slug: String,
        id: String,
        prompt: String,
        target: String,
    },
    ToolOutputChunk {
        instance_slug: String,
        chat_id: String,
        chunk: String,
    },
    /// Tool result arrived for an MCP App — viewer should send it to the iframe.
    McpAppResult {
        instance_slug: String,
        chat_id: String,
        /// The message id of the McpApp chat message to update.
        message_id: String,
        tool_output: String,
    },
    /// Streaming: an MCP App tool call is starting — show the iframe immediately.
    McpAppStart {
        instance_slug: String,
        chat_id: String,
        tool_name: String,
        html: String,
    },
    /// Server-generated TTS audio ready for playback.
    ChatAudioReady {
        instance_slug: String,
        chat_id: String,
        /// Base64-encoded MP3 audio bytes.
        audio_base64: String,
        /// Message IDs that this audio covers (for word reveal).
        message_ids: Vec<String>,
    },
    /// Streaming: partial tool arguments delta for an MCP App.
    McpAppInputDelta {
        instance_slug: String,
        chat_id: String,
        delta: String,
    },
    /// Automatic memory recall — shows which memories were injected into context.
    MemoryRecall {
        instance_slug: String,
        chat_id: String,
        memories: Vec<RecalledMemory>,
    },
    /// Full chat state snapshot — client reconciles against this.
    ChatSnapshot {
        instance_slug: String,
        chat_id: String,
        messages: Vec<ChatMessage>,
        agent_running: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct RecalledMemory {
    pub path: String,
    pub preview: String,
    pub score: f32,
}
