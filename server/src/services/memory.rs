use std::path::Path;
use std::sync::Mutex;

use crate::domain::chat::ChatMessage;
use crate::domain::memory::{MemoryEntry, MemoryGraph};
use crate::services::llm::LlmBackend;

// ═══════════════════════════════════════════════════════════════════════════
// Frontmatter — timestamps for temporal awareness
// ═══════════════════════════════════════════════════════════════════════════

/// Parsed frontmatter from a memory file.
pub struct Frontmatter {
    pub created: Option<String>,
    pub updated: Option<String>,
}

/// Parse YAML frontmatter from memory file content.
/// Returns (frontmatter, body) where body is the content without frontmatter.
pub fn parse_frontmatter(content: &str) -> (Frontmatter, &str) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (
            Frontmatter {
                created: None,
                updated: None,
            },
            content,
        );
    }

    // Find closing ---
    if let Some(end) = trimmed[3..].find("\n---") {
        let yaml = &trimmed[3..3 + end];
        let body_start = 3 + end + 4; // skip closing "---"
        let body = trimmed[body_start..].trim_start_matches('\n');

        let mut created = None;
        let mut updated = None;
        for line in yaml.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("created:") {
                created = Some(val.trim().to_string());
            } else if let Some(val) = line.strip_prefix("updated:") {
                updated = Some(val.trim().to_string());
            }
        }

        (Frontmatter { created, updated }, body)
    } else {
        (
            Frontmatter {
                created: None,
                updated: None,
            },
            content,
        )
    }
}

/// Add or update frontmatter timestamps on memory content.
/// For new files: adds created + updated. For existing: updates the updated field.
pub fn stamp_content(content: &str, existing_content: Option<&str>) -> String {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

    if let Some(existing) = existing_content {
        // Updating existing file — preserve created, update updated
        let (fm, _) = parse_frontmatter(existing);
        let created = fm.created.unwrap_or_else(|| today.clone());
        format!("---\ncreated: {created}\nupdated: {today}\n---\n{content}")
    } else {
        // New file
        format!("---\ncreated: {today}\nupdated: {today}\n---\n{content}")
    }
}

/// Format a YYYY-MM-DD date as short display (Mar 28).
fn format_date_short(date: &str) -> String {
    if let Ok(d) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        d.format("%b %d").to_string()
    } else {
        date.to_string()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Frozen catalog — cached in RAM, survives memory writes within a session.
// Only refreshed on context clear, compaction, or server restart.
// ═══════════════════════════════════════════════════════════════════════════

static FROZEN_CATALOG: Mutex<Option<std::collections::HashMap<String, String>>> = Mutex::new(None);

/// Get the frozen memory catalog for an instance.
/// First call loads from disk and caches; subsequent calls return the cached version.
/// This prevents system prompt changes (and cache invalidation) when memories are written.
#[allow(dead_code)]
pub fn get_frozen_catalog(media: &super::media_text::MediaStore, instance_slug: &str) -> String {
    let key = instance_slug.to_string();
    let mut guard = FROZEN_CATALOG.lock().unwrap();
    let map = guard.get_or_insert_with(std::collections::HashMap::new);
    map.entry(key)
        .or_insert_with(|| load_catalog_snapshot(media, instance_slug))
        .clone()
}

/// Invalidate the frozen catalog for an instance.
/// Call after context clear, compaction, or catalog rebuild.
pub fn invalidate_frozen_catalog(instance_slug: &str) {
    let mut guard = FROZEN_CATALOG.lock().unwrap();
    if let Some(map) = guard.as_mut() {
        map.remove(instance_slug);
    }
}

/// Strictly scan the memory library through the persistent workspace capability.
pub fn scan_library_checked(
    media: &super::media_text::MediaStore,
    instance_slug: &str,
) -> Result<Vec<MemoryEntry>, String> {
    let mut entries = Vec::new();
    for path in media
        .memory_files(instance_slug)
        .map_err(|error| format!("scan memory directory: {error}"))?
    {
        let metadata = media
            .memory_metadata(instance_slug, &path)
            .map_err(|error| format!("read memory metadata {path}: {error}"))?;
        if let Some(source_type) = super::media_text::source_type(&path) {
            let kind = source_type.strip_prefix("media_").unwrap_or("file");
            let extension = Path::new(&path)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            entries.push(MemoryEntry {
                path,
                summary: format!("[{kind}: {extension}]"),
                size: usize::try_from(metadata.len).unwrap_or(usize::MAX),
            });
            continue;
        }
        let content = media
            .read_memory_text(instance_slug, &path)
            .map_err(|error| format!("read memory text {path}: {error}"))?;
        let (frontmatter, body) = parse_frontmatter(&content);
        let date_prefix = frontmatter
            .updated
            .or(frontmatter.created)
            .map(|date| format!("({}) ", format_date_short(&date)))
            .unwrap_or_default();
        let summary_text = body
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("")
            .trim()
            .chars()
            .take(120)
            .collect::<String>();
        entries.push(MemoryEntry {
            path,
            summary: format!("{date_prefix}{summary_text}"),
            size: content.len(),
        });
    }
    Ok(entries)
}

pub fn scan_library(
    media: &super::media_text::MediaStore,
    instance_slug: &str,
) -> Vec<MemoryEntry> {
    scan_library_checked(media, instance_slug).unwrap_or_default()
}

/// Rebuild and persist the memory catalog snapshot to disk.
/// Call this after context clear or compaction — not on every request.
#[allow(dead_code)]
pub fn rebuild_catalog_snapshot(instance_slug: &str, media: &super::media_text::MediaStore) {
    let entries = scan_library(media, instance_slug);
    if entries.is_empty() {
        return;
    }

    // Separate pinned memories (full content in prompt) from regular (catalog only)
    let (pinned, regular): (Vec<_>, Vec<_>) =
        entries.iter().partition(|e| e.path.starts_with("pinned/"));

    let mut prompt = format!(
        "## memory\nyou have a personal memory library. use `memory_read` to read memories when relevant.\n"
    );

    // Pinned memories: always-loaded, full content inline
    if !pinned.is_empty() {
        prompt.push_str("\n### pinned (always loaded)\n");
        for entry in &pinned {
            let content = media
                .read_memory_text(instance_slug, &entry.path)
                .unwrap_or_default();
            prompt.push_str(&format!("\n**{}**\n{}\n", entry.path, content.trim()));
        }
    }

    // Regular catalog: paths + summaries
    if !regular.is_empty() {
        prompt.push_str(&format!("\ncatalog ({} files):\n", regular.len()));
        for entry in &regular {
            prompt.push_str(&format!("- {} — {}\n", entry.path, entry.summary));
        }
    }

    prompt.push_str(
        "\nuse these memories naturally — `memory_read` what you need. \
         don't announce that you remember — just know.",
    );

    if let Err(e) = media.write_instance_text(instance_slug, "memory_catalog.txt", &prompt) {
        log::warn!("[memory] failed to write catalog snapshot: {e}");
    } else {
        log::info!(
            "[memory] catalog snapshot rebuilt: {} pinned, {} catalog",
            pinned.len(),
            regular.len()
        );
    }
}

/// Load the static memory catalog snapshot from disk.
/// Returns empty string if no snapshot exists yet (first boot / pre-compaction).
#[allow(dead_code)]
pub fn load_catalog_snapshot(media: &super::media_text::MediaStore, instance_slug: &str) -> String {
    media
        .read_instance_text(instance_slug, "memory_catalog.txt", 4 * 1024 * 1024)
        .unwrap_or_default()
}

/// Build a full library catalog for memory maintenance (heartbeat).
/// Shows every file path, size, and first-line summary.
pub fn build_library_catalog(media: &super::media_text::MediaStore, instance_slug: &str) -> String {
    let entries = scan_library(media, instance_slug);
    if entries.is_empty() {
        return String::from("(empty library)");
    }

    let total_files = entries.len();

    let mut result = format!("{total_files} files:\n");
    for entry in &entries {
        result.push_str(&format!("- {} — {}\n", entry.path, entry.summary));
    }
    result
}

// ═══════════════════════════════════════════════════════════════════════════
// Memory graph — undirected connections between memory files
// ═══════════════════════════════════════════════════════════════════════════

/// Load the memory graph from disk. Returns empty graph if file doesn't exist.
pub fn load_graph(media: &super::media_text::MediaStore, instance_slug: &str) -> MemoryGraph {
    match media.read_instance_text(instance_slug, "memory_graph.json", 4 * 1024 * 1024) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => MemoryGraph::default(),
    }
}

/// Save the memory graph to disk.
pub fn save_graph(media: &super::media_text::MediaStore, instance_slug: &str, graph: &MemoryGraph) {
    if let Ok(json) = serde_json::to_string_pretty(graph) {
        if let Err(e) = media.write_instance_text(instance_slug, "memory_graph.json", &json) {
            log::warn!("[graph] failed to write memory_graph.json: {e}");
        }
    }
}

/// Normalize an edge to a sorted pair (for deduplication).
fn sorted_edge(a: &str, b: &str) -> [String; 2] {
    if a <= b {
        [a.to_string(), b.to_string()]
    } else {
        [b.to_string(), a.to_string()]
    }
}

/// Add an edge between two memory paths. Returns true if the edge was new.
pub fn add_edge(
    media: &super::media_text::MediaStore,
    instance_slug: &str,
    a: &str,
    b: &str,
) -> bool {
    if a == b {
        return false;
    }
    let mut graph = load_graph(media, instance_slug);
    let edge = sorted_edge(a, b);
    if graph.edges.iter().any(|e| *e == edge) {
        return false;
    }
    graph.edges.push(edge);
    save_graph(media, instance_slug, &graph);
    log::info!("[graph] added edge: {} <-> {} for {instance_slug}", a, b);
    true
}

/// Remove all edges involving a given path (used when deleting a memory file).
pub fn remove_edges_for_path(
    media: &super::media_text::MediaStore,
    instance_slug: &str,
    path: &str,
) {
    let mut graph = load_graph(media, instance_slug);
    let before = graph.edges.len();
    graph.edges.retain(|e| e[0] != path && e[1] != path);
    if graph.edges.len() != before {
        save_graph(media, instance_slug, &graph);
        log::info!(
            "[graph] removed {} edges for deleted path: {path}",
            before - graph.edges.len()
        );
    }
}

/// Get all neighbors (connected paths) for a given path.
pub fn get_neighbors(graph: &MemoryGraph, path: &str) -> Vec<String> {
    let mut neighbors = Vec::new();
    for edge in &graph.edges {
        if edge[0] == path {
            neighbors.push(edge[1].clone());
        } else if edge[1] == path {
            neighbors.push(edge[0].clone());
        }
    }
    neighbors
}

/// Run legacy migration through the persistent workspace capability.
pub fn migrate_all_instances(media: &super::media_text::MediaStore) {
    let Ok(slugs) = media.instance_slugs() else {
        return;
    };
    for slug in slugs {
        migrate_legacy_memory(media, &slug);
    }
}

/// Migrate legacy memory format (facts.md + episodes.md) into the new library structure.
pub fn migrate_legacy_memory(media: &super::media_text::MediaStore, instance_slug: &str) {
    if media
        .memory_exists(instance_slug, ".migrated")
        .unwrap_or(false)
    {
        return;
    }
    let _ = media.ensure_memory_dir(instance_slug);

    if let Ok(content) = media.read_memory_text(instance_slug, "facts.md") {
        let mut current_category = String::from("general");
        let mut categories: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for line in content.lines() {
            let line = line.trim();
            if let Some(category) = line.strip_prefix("## ") {
                current_category = category.trim().to_lowercase();
            } else if let Some(fact) = line.strip_prefix("- ") {
                categories
                    .entry(current_category.clone())
                    .or_default()
                    .push(fact.to_owned());
            }
        }
        for (category, facts) in &categories {
            if let Err(error) = media.write_memory_text(
                instance_slug,
                &format!("facts/{category}.md"),
                &facts
                    .iter()
                    .map(|fact| format!("- {fact}\n"))
                    .collect::<String>(),
            ) {
                log::warn!("failed to migrate facts/{category}.md: {error}");
            }
        }
        let _ = media.rename_memory(instance_slug, "facts.md", "_legacy_facts.md");
    }

    if let Ok(content) = media.read_memory_text(instance_slug, "episodes.md") {
        let mut episode_idx = 0_u32;
        let mut lines = content.lines().peekable();
        while let Some(line) = lines.next() {
            let Some(main) = line.trim().strip_prefix("- ") else {
                continue;
            };
            let (content_part, emotion) = if let Some(position) = main.rfind("(felt: ") {
                (
                    main[..position].trim().to_owned(),
                    main[position + 7..].trim_end_matches(')').trim().to_owned(),
                )
            } else {
                (main.to_owned(), String::new())
            };
            let significance = lines
                .peek()
                .filter(|next| next.trim_start().starts_with("why: "))
                .map(|next| next.trim().trim_start_matches("why: ").to_owned())
                .unwrap_or_default();
            if !significance.is_empty() {
                lines.next();
            }
            let name = content_part
                .to_lowercase()
                .chars()
                .map(|character| {
                    if character.is_alphanumeric() || character == ' ' {
                        character
                    } else {
                        ' '
                    }
                })
                .collect::<String>()
                .split_whitespace()
                .take(5)
                .collect::<Vec<_>>()
                .join("-");
            let name = if name.is_empty() {
                format!("moment-{episode_idx}")
            } else {
                name
            };
            let mut body = content_part;
            if !emotion.is_empty() {
                body.push_str(&format!("\n\nfelt: {emotion}"));
            }
            if !significance.is_empty() {
                body.push_str(&format!("\nwhy: {significance}"));
            }
            if let Err(error) =
                media.write_memory_text(instance_slug, &format!("moments/{name}.md"), &body)
            {
                log::warn!("failed to migrate moment {name}: {error}");
            }
            episode_idx += 1;
        }
        let _ = media.rename_memory(instance_slug, "episodes.md", "_legacy_episodes.md");
    }

    if media
        .memory_exists(instance_slug, "memory.db")
        .unwrap_or(false)
    {
        let _ = media.rename_memory(instance_slug, "memory.db", "_legacy_memory.db");
    }
    let _ = media.write_memory_text(instance_slug, ".migrated", "migrated");
}

/// Extract new memories from recent messages and store them in the library.
/// Called as a background task after each chat turn.
pub async fn extract_and_store(
    instance_slug: &str,
    recent_messages: &[ChatMessage],
    llm: &LlmBackend,
    vector_store: &super::vector::VectorStore,
) -> anyhow::Result<()> {
    let media = vector_store.media_store();
    media.ensure_memory_dir(instance_slug)?;

    // Build existing library context
    let entries = scan_library(&media, instance_slug);
    let file_count = entries.len();
    let existing_summary = if entries.is_empty() {
        String::from("(empty library — no memories yet)")
    } else {
        let mut s = String::new();
        for entry in &entries {
            let content = if super::media_text::source_type(&entry.path).is_some() {
                media.read(instance_slug, &entry.path).unwrap_or_default()
            } else {
                media
                    .read_memory_text(instance_slug, &entry.path)
                    .unwrap_or_default()
            };
            s.push_str(&format!("[{}]\n{}\n\n", entry.path, content.trim()));
        }
        // Truncate if too long (find a char boundary to avoid panic)
        if s.len() > 4000 {
            let mut end = 4000;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
            s.push_str("\n...(truncated)");
        }
        s
    };

    // Detect image attachments in messages
    let attachment_re = regex::Regex::new(r"\[attached:\s*(.+?)\s*\(([^)]+)\)\]").unwrap();

    let mut image_uploads: Vec<(String, String)> = Vec::new(); // (upload_id, original_name)
    let conversation = recent_messages
        .iter()
        .map(|m| {
            let role = match m.role {
                crate::domain::chat::ChatRole::User => "user",
                crate::domain::chat::ChatRole::Assistant => "assistant",
            };
            // Collect image attachment IDs
            for cap in attachment_re.captures_iter(&m.content) {
                let name = cap[1].to_string();
                let upload_id = cap[2].to_string();
                if media
                    .upload_mime_type(instance_slug, &upload_id)
                    .ok()
                    .flatten()
                    .is_some_and(|mime| mime.starts_with("image/"))
                {
                    image_uploads.push((upload_id.clone(), name.clone()));
                }
            }
            format!("{role}: {}", m.content)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let image_context = if image_uploads.is_empty() {
        String::new()
    } else {
        let list = image_uploads
            .iter()
            .map(|(id, name)| format!("  - \"{name}\" (upload_id: {id})"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\nimages shared in this conversation:\n{list}\n\
             you can save important images using: {{\"action\": \"save_image\", \"upload_id\": \"...\", \"path\": \"folder/name.jpg\", \"description\": \"what this image shows\"}}\n\
             only save images that are meaningful — personal photos, important screenshots, etc. NOT memes or random links.\n"
        )
    };

    let extraction_prompt = format!(
        r#"analyze this conversation and decide what to remember.

your memory library currently contains:
{existing_summary}

recent conversation:
{conversation}
{image_context}
respond with JSON — an array of file operations:
{{
  "ops": [
    {{"action": "write", "path": "folder/file.md", "content": "the memory content"}},
    {{"action": "append", "path": "folder/file.md", "content": "additional info to add"}},
    {{"action": "delete", "path": "folder/file.md"}},
    {{"action": "save_image", "upload_id": "upload_xxx", "path": "moments/photo.jpg", "description": "what this image shows"}}
  ]
}}

rules:
- organize memories into folders by topic (e.g. about/, preferences/, moments/, projects/)
- each file should cover one coherent topic or moment
- file names should be descriptive kebab-case (e.g. "about/work.md", "moments/late-night-debugging.md")
- use "write" to create new files or replace outdated ones
- use "append" to add new info to an EXISTING file (prefer this over creating new files)
- use "delete" to remove files with outdated/wrong info
- use "save_image" to save meaningful images shared in the conversation (only if images were shared)
- DON'T duplicate info that's already in the library
- DON'T force it — most conversations produce 0-1 ops
- DON'T create a new file if you can append to an existing one on the same topic
- keep files concise — a few lines each, not essays
- NEVER create a write or append op with empty content — every write/append MUST have non-empty content
- there are currently {file_count} files. aim for quality over quantity — merge related topics

do NOT save images unless they are clearly meaningful (personal photos, important screenshots). ignore memes, random links, UI screenshots.

## memory graph
you can also create connections between related memories using the "connect" action:
{{"action": "connect", "from": "about/work.md", "to": "schedule/meetings.md"}}
this creates an undirected edge in the memory graph — meaning these two facts are related.
examples of good connections:
- "Тимур учится в 9:25" <-> "Тимур встает в 8" (schedule implies routine)
- "любит кофе" <-> "утренние привычки" (preference relates to habit)
- "работает в компании X" <-> "проект Y" (context connects)
only connect memories that are meaningfully related. don't over-connect."#
    );

    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "ops": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["write", "append", "delete", "save_image", "connect"]
                        },
                        "path": { "type": "string" },
                        "content": { "type": "string" },
                        "upload_id": { "type": "string" },
                        "description": { "type": "string" },
                        "from": { "type": "string" },
                        "to": { "type": "string" }
                    },
                    "required": ["action", "path", "content"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["ops"],
        "additionalProperties": false
    });

    let (response, _) = llm
        .chat_json(
            "you are a memory librarian. you organize memories into a clean file-based library. \
             you understand the difference between facts (knowing something) and moments (shared experiences). \
             you can also save images that are meaningful to the user.",
            &extraction_prompt,
            schema,
        )
        .await?;

    let ops: Vec<MemoryOp> = match serde_json::from_str::<MemoryOps>(&response) {
        Ok(m) => m.ops,
        Err(e) => {
            log::warn!("memory: failed to parse structured output: {e}");
            parse_memory_ops(&response)
        }
    };
    apply_memory_ops(instance_slug, &ops, vector_store).await
}

async fn apply_memory_ops(
    instance_slug: &str,
    ops: &[MemoryOp],
    vector_store: &super::vector::VectorStore,
) -> anyhow::Result<()> {
    for op in ops {
        // Sanitize path — allow image extensions for save_image
        let clean_path = if op.action == "save_image"
            || (op.action == "delete" && super::media_text::source_type(&op.path).is_some())
        {
            sanitize_media_path(&op.path)
        } else {
            sanitize_memory_path(&op.path)
        };
        if clean_path.is_empty() {
            continue;
        }
        match op.action.as_str() {
            "write" => {
                if op.content.trim().is_empty() {
                    log::warn!("memory: skipping empty write to {clean_path}");
                    continue;
                }
                vector_store
                    .write_text_memory(instance_slug, &clean_path, &op.content, false)
                    .await
                    .map_err(anyhow::Error::msg)?;
                log::info!("memory: wrote {clean_path} for {instance_slug}");
            }
            "append" => {
                if op.content.trim().is_empty() {
                    log::warn!("memory: skipping empty append to {clean_path}");
                    continue;
                }
                vector_store
                    .write_text_memory(instance_slug, &clean_path, &op.content, true)
                    .await
                    .map_err(anyhow::Error::msg)?;
                log::info!("memory: appended to {clean_path} for {instance_slug}");
            }
            "delete" => {
                let deletion_error = vector_store
                    .delete_memory(instance_slug, &clean_path)
                    .await
                    .err();
                remove_edges_for_path(&vector_store.media_store(), instance_slug, &clean_path);
                if deletion_error.is_none() {
                    log::info!("memory: reconciled deletion of {clean_path} for {instance_slug}");
                } else {
                    return Err(anyhow::anyhow!(
                        "cleanup incomplete for {clean_path}: {}",
                        deletion_error.unwrap()
                    ));
                }
            }
            "connect" => {
                let from = &op.from;
                let to = &op.to;
                if !from.is_empty() && !to.is_empty() {
                    add_edge(&vector_store.media_store(), instance_slug, from, to);
                }
            }
            "save_image" => {
                let upload_id = if !op.upload_id.is_empty() {
                    &op.upload_id
                } else {
                    log::warn!("memory: save_image — missing upload_id");
                    continue;
                };

                let content = if op.content.trim().is_empty() {
                    &op.description
                } else {
                    &op.content
                };
                match vector_store
                    .replace_media_from_upload(instance_slug, &clean_path, upload_id, content)
                    .await
                {
                    Ok(Some(error)) => {
                        log::warn!("memory: image saved; semantic indexing pending: {error}")
                    }
                    Ok(None) => {
                        log::info!("memory: saved image {clean_path} for {instance_slug}")
                    }
                    Err(error) => {
                        log::warn!("memory: save_image publication failed: {error}");
                        continue;
                    }
                }
            }
            _ => {
                log::warn!("memory: unknown action '{}' for {instance_slug}", op.action);
            }
        }
    }

    Ok(())
}

/// Sanitize a memory file path to prevent directory traversal.
fn sanitize_memory_path(path: &str) -> String {
    let path = path.trim().trim_start_matches('/');
    // Reject any path component that is ".." or starts with "."
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == ".." || p.starts_with('.'))
    {
        return String::new();
    }
    // Ensure .md extension
    let result = parts.join("/");
    if !result.ends_with(".md") {
        format!("{result}.md")
    } else {
        result
    }
}

/// Sanitize path for image/media files — allows image extensions.
fn sanitize_media_path(path: &str) -> String {
    let path = path.trim();
    let parts: Vec<&str> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == ".." || p.starts_with('.'))
    {
        return String::new();
    }
    let result = parts.join("/");
    let lower = result.to_lowercase();
    if lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".png")
        || lower.ends_with(".webp")
        || lower.ends_with(".gif")
        || lower.ends_with(".mp4")
        || lower.ends_with(".mp3")
        || lower.ends_with(".wav")
    {
        result
    } else {
        format!("{result}.jpg")
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct MemoryOp {
    action: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    content: String,
    /// Upload ID for save_image action.
    #[serde(default)]
    upload_id: String,
    /// Agent-authored image description, persisted as the v1 representation.
    #[serde(default)]
    description: String,
    /// Source path for connect action.
    #[serde(default)]
    from: String,
    /// Target path for connect action.
    #[serde(default)]
    to: String,
}

#[derive(serde::Deserialize)]
struct MemoryOps {
    #[serde(default)]
    ops: Vec<MemoryOp>,
}

fn parse_memory_ops(response: &str) -> Vec<MemoryOp> {
    // Try direct parse
    if let Ok(m) = serde_json::from_str::<MemoryOps>(response) {
        return m.ops;
    }

    // Try stripping markdown code fences
    let trimmed = response.trim();
    let json_str = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };

    if let Ok(m) = serde_json::from_str::<MemoryOps>(json_str) {
        return m.ops;
    }

    Vec::new()
}

#[cfg(test)]
mod strict_scan_tests {
    use super::*;

    #[test]
    fn derived_graph_and_catalog_roundtrip_through_media_store() {
        let workspace = tempfile::tempdir().unwrap();
        let memory = workspace.path().join("instances/one/memory");
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(memory.join("note.md"), "Orion").unwrap();
        let media = super::super::media_text::MediaStore::open(workspace.path()).unwrap();

        rebuild_catalog_snapshot("one", &media);
        assert!(load_catalog_snapshot(&media, "one").contains("note.md"));

        assert!(add_edge(&media, "one", "note.md", "other.md"));
        assert_eq!(
            get_neighbors(&load_graph(&media, "one"), "note.md"),
            ["other.md"]
        );
        remove_edges_for_path(&media, "one", "note.md");
        assert!(load_graph(&media, "one").edges.is_empty());
    }

    #[test]
    fn checked_scan_preserves_filtering_and_sorting() {
        let workspace = std::env::temp_dir().join(format!("memory-scan-{}", uuid::Uuid::new_v4()));
        let memory = workspace.join("instances/slug/memory");
        std::fs::create_dir_all(memory.join("nested")).unwrap();
        std::fs::write(memory.join("z.md"), "z").unwrap();
        std::fs::write(memory.join("nested/a.md"), "a").unwrap();
        std::fs::write(memory.join(".hidden.md"), "hidden").unwrap();
        std::fs::write(memory.join("ignored.txt"), "ignored").unwrap();

        let media = super::super::media_text::MediaStore::open(&workspace).unwrap();
        let entries = scan_library_checked(&media, "slug").unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            ["nested/a.md", "z.md"]
        );
        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn checked_scan_reports_invalid_utf8_text() {
        let workspace = std::env::temp_dir().join(format!("memory-scan-{}", uuid::Uuid::new_v4()));
        let memory = workspace.join("instances/slug/memory");
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(memory.join("broken.md"), [0xff]).unwrap();
        let media = super::super::media_text::MediaStore::open(&workspace).unwrap();
        assert!(scan_library_checked(&media, "slug").is_err());
        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn checked_scan_reports_unreadable_directory_where_enforced() {
        use std::os::unix::fs::PermissionsExt;
        let workspace = std::env::temp_dir().join(format!("memory-scan-{}", uuid::Uuid::new_v4()));
        let memory = workspace.join("instances/slug/memory/blocked");
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::set_permissions(&memory, std::fs::Permissions::from_mode(0o000)).unwrap();
        let media = super::super::media_text::MediaStore::open(&workspace).unwrap();
        let result = scan_library_checked(&media, "slug");
        std::fs::set_permissions(&memory, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(workspace).unwrap();
        // Root can bypass mode bits in some CI containers.
        if let Err(error) = result {
            assert!(error.contains("scan memory directory"));
        }
    }
}

#[cfg(test)]
mod media_representation_tests {
    use super::*;

    #[test]
    fn media_sidecars_are_reserved_and_filtered_from_catalog() {
        let workspace = tempfile::tempdir().unwrap();
        let dir = workspace.path().join("instances/one/memory");
        std::fs::create_dir_all(&dir).unwrap();
        for path in ["photo.PNG", "photo.PNG.md", "orphan.mp3.md", "note.md"] {
            std::fs::write(dir.join(path), "description").unwrap();
        }
        let media = super::super::media_text::MediaStore::open(workspace.path()).unwrap();
        for entries in [
            scan_library(&media, "one"),
            scan_library_checked(&media, "one").unwrap(),
        ] {
            assert_eq!(
                entries.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
                ["note.md", "photo.PNG"]
            );
        }
    }
    #[tokio::test]
    async fn media_extracted_save_image_uses_persisted_description_and_delete_cleans_it() {
        use crate::services::embedding::tests::{MockServer, response};
        let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
        let ws = tempfile::tempdir().unwrap();
        let uploads = ws.path().join("instances/one/uploads");
        std::fs::create_dir_all(&uploads).unwrap();
        std::fs::write(uploads.join("raw.jpg"), [0xff]).unwrap();
        std::fs::write(
            uploads.join("upload_one.json"),
            r#"{"stored_name":"raw.jpg"}"#,
        )
        .unwrap();
        let store =
            super::super::vector::VectorStore::connect_with_config(ws.path(), &mock.config).await;
        let ops = parse_memory_ops(
            r#"{"ops":[{"action":"save_image","path":"photo.jpg","upload_id":"upload_one","description":"Orion sky"}]}"#,
        );
        apply_memory_ops("one", &ops, &store).await.unwrap();
        let dir = ws.path().join("instances/one/memory");
        assert_eq!(
            super::super::media_text::read(&dir, "photo.jpg").unwrap(),
            "Orion sky"
        );
        assert_eq!(
            store.list_all("one", 10).await.unwrap()[0].path,
            "photo.jpg"
        );
        let ops = parse_memory_ops(r#"{"ops":[{"action":"delete","path":"photo.jpg"}]}"#);
        apply_memory_ops("one", &ops, &store).await.unwrap();
        assert!(!dir.join("photo.jpg").exists());
        assert!(!dir.join("photo.jpg.md").exists());
        assert!(store.list_all("one", 10).await.unwrap().is_empty());
        store
            .backfill_text_memories(ws.path(), "one")
            .await
            .unwrap();
        let ops = parse_memory_ops(
            r#"{"ops":[{"action":"save_image","path":"photo.jpg","upload_id":"upload_one"}]}"#,
        );
        apply_memory_ops("one", &ops, &store).await.unwrap();
        assert!(dir.join("photo.jpg").exists());
        assert!(store.needs_backfill("one").await.unwrap());
        assert_eq!(mock.requests.lock().unwrap().len(), 1);
    }
}
