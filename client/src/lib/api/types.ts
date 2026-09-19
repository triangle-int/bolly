export type ChatRole = "user" | "assistant";

export type MessageKind = "message" | "tool_call" | "tool_output" | "mcp_app" | "compaction";

export interface ChatMessage {
	id: string;
	role: ChatRole;
	content: string;
	created_at: string;
	kind?: MessageKind;
	tool_name?: string;
	mcp_app_html?: string;
	mcp_app_input?: string;
	model?: string;
}

export interface ChatRequest {
	instance_slug: string;
	content: string;
	chat_id?: string;
}

export interface ChatResponse {
	instance_slug: string;
	chat_id: string;
	messages: ChatMessage[];
	agent_running: boolean;
}

export interface ChatSummary {
	id: string;
	title: string;
	message_count: number;
	last_message_at: string | null;
	created_at: string;
}

export interface InstanceSummary {
	slug: string;
	companion_name: string;
	soul_exists: boolean;
	drops_count: number;
	has_memory: boolean;
	has_skin: boolean;
}

export interface LlmSummary {
	model: string | null;
	configured: boolean;
}

export interface ServerMeta {
	app: string;
	version: string;
	commit: string;
	port: number;
	workspace_dir: string;
	instances_count: number;
	skills_count: number;
	llm: LlmSummary;
}

export interface UpdateLlmRequest {
	api_key: string;
}

export interface Soul {
	content: string;
	exists: boolean;
}

export interface SoulTemplate {
	id: string;
	name: string;
	description: string;
	content: string;
}

export type DropKind =
	| "thought"
	| "idea"
	| "poem"
	| "observation"
	| "reflection"
	| "recommendation"
	| "story"
	| "question"
	| "note";

export interface Drop {
	id: string;
	kind: DropKind;
	title: string;
	content: string;
	mood: string;
	created_at: string;
	image_url?: string;
}

export interface Thought {
	id: string;
	raw: string;
	actions: string[];
	mood: string;
	created_at: string;
}


export interface ChildAgent {
	name: string;
	description: string;
	prompt: string;
	interval_hours: number;
	model: string;
	triage: boolean;
	tools: boolean;
	enabled: boolean;
	tool_groups: string[];
	last_run: number;
	is_due: boolean;
	is_builtin: boolean;
	modified_fields: string[];
}

export interface AgentHistoryEntry {
	content: string;
	timestamp: string;
	id: string;
}

export interface AgentRunSummary {
	id: string;
	agent_name: string;
	agent_kind: 'scheduled' | 'on_demand';
	trigger: string;
	started_at: number;
	finished_at: number;
	duration_ms: number;
	tokens_used: number;
	model: string;
	summary: string;
	status: 'completed' | { failed: { error: string } };
}

export interface AgentRun extends AgentRunSummary {
	trace: unknown[];
}

export interface SkillSource {
	repo: string;
	version: string;
}

export interface Skill {
	id: string;
	name: string;
	description: string;
	icon: string;
	builtin: boolean;
	enabled: boolean;
	kind?: "local" | "anthropic";
	anthropic_skill_id?: string;
	instructions: string;
	source?: SkillSource;
	resources?: string[];
}

export interface RegistryEntry {
	id: string;
	name: string;
	description: string;
	icon: string;
	repo: string;
	git_ref: string;
	author: string;
	path: string;
	installed: boolean;
}

export interface UploadMeta {
	id: string;
	original_name: string;
	stored_name: string;
	mime_type: string;
	size: number;
	uploaded_at: string;
}

export interface ContextSection {
	name: string;
	chars: number;
	tokens: number;
}

export interface ContextStats {
	system_prompt: ContextSection[];
	system_prompt_total_tokens: number;
	tools: string[];
	tools_count: number;
	tools_tokens_estimate: number;
	history_messages: number;
	history_tokens_estimate: number;
	total_input_tokens_estimate: number;
}

export interface HeartbeatUpdate {
	id: string;
	description: string;
	preview: string;
}

export interface Stats {
	hourly_activity: number[];
	daily_activity: number[];
	total_messages: number;
	avg_message_length: number;
	avg_response_interval_secs: number;
	daily_history: [string, number][];
	mood_counts: Record<string, number>;
	streak_days: number;
	first_message_at: string | null;
}

export interface MemoryEntry {
	path: string;
	summary: string;
	size: number;
}

export interface MemoryGraph {
	edges: [string, string][];
}

export type ServerEvent =
	| {
			type: "chat_message_created";
			instance_slug: string;
			chat_id: string;
			message: ChatMessage;
	  }
	| {
			type: "instance_discovered";
			instance: InstanceSummary;
	  }
	| {
			type: "mood_updated";
			instance_slug: string;
			mood: string;
	  }
	| {
			type: "agent_running";
			instance_slug: string;
			chat_id: string;
	  }
	| {
			type: "agent_stopped";
			instance_slug: string;
			chat_id: string;
	  }
	| {
			type: "tool_activity";
			instance_slug: string;
			chat_id: string;
			tool_name: string;
			summary: string;
	  }
	| {
			type: "drop_created";
			instance_slug: string;
			drop: Drop;
	  }
	| {
			type: "heartbeat_thought";
			instance_slug: string;
			thought: Thought;
	  }
	| {
			type: "context_compacting";
			instance_slug: string;
			chat_id: string;
			messages_compacted: number;
	  }
	| {
			type: "chat_stream_delta";
			instance_slug: string;
			chat_id: string;
			message_id: string;
			delta: string;
	  }
	| {
			type: "secret_request";
			instance_slug: string;
			id: string;
			prompt: string;
			target: string;
	  }
	| {
			type: "tool_output_chunk";
			instance_slug: string;
			chat_id: string;
			chunk: string;
	  }
	| {
			type: "mcp_app_result";
			instance_slug: string;
			chat_id: string;
			message_id: string;
			tool_output: string;
	  }
	| {
			type: "mcp_app_start";
			instance_slug: string;
			chat_id: string;
			tool_name: string;
			html: string;
	  }
	| {
			type: "chat_audio_ready";
			instance_slug: string;
			chat_id: string;
			audio_base64: string;
			message_ids: string[];
	  }
	| {
			type: "mcp_app_input_delta";
			instance_slug: string;
			chat_id: string;
			delta: string;
	  }
	| {
			type: "import_progress";
			instance_slug: string;
			stage: "parsing" | "extracting" | "organizing" | "writing" | "done" | "error";
			detail: string;
	  }
	| {
			type: "memory_recall";
			instance_slug: string;
			memories: { path: string; preview: string; score: number }[];
	  }
	| {
			type: "computer_use_request";
			instance_slug: string;
			request_id: string;
			action: string;
			coordinate?: [number, number];
			text?: string;
			key?: string;
			scroll_delta?: [number, number];
	  }
;
