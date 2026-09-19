# Extensions

Nolune stays extensible without becoming a generic agent-framework control
panel (#97). Two kinds of extension exist, and both are bounded.

## Skills

Skills are reviewed, installable bundles from the registry (Settings → Skills →
Browse). The consumer UI no longer offers a "New skill" prompt-authoring
flow, and the built-in "Skill Creator" is gone. Developers author skills by
placing a folder under `~/.nolune/skills/<id>/` with a `SKILL.md`; the server
lists it like any installed skill and it can be removed from the UI.

## MCP servers

Every MCP server carries a trust label and an exact tool grant in
`config.toml`:

```toml
[[mcp_servers]]
name = "brave-search"
url = "https://mcp.bravesearch.com/sse"
trust = "curated"                 # or "custom"
enabled_tools = ["brave_web_search"]
[mcp_servers.headers]
Authorization = "Bearer …"        # this server's secret only
```

| Trust | How it gets there | Default grant |
| --- | --- | --- |
| `curated` | one-click install from the reviewed catalog (`GET /api/config/mcp/suggested`); name **and** URL must match the catalog exactly | every discovered tool is enabled |
| `custom` | Settings → Extensions → **Advanced** → *Connect a custom server*, or editing `config.toml` (the only way to configure stdio/command servers) | **no tools** until the user enables them one by one |

`POST /api/config/mcp` refuses a non-catalog server unless the body carries
`"acknowledge_untrusted": true`; the Advanced form sends it only after the
user ticks the acknowledgement. Legacy entries without a `trust` field load
as `custom` with an empty grant, so an existing unreviewed server keeps its
connection but exposes no tools until reviewed.

### Exact grants

`PUT /api/config/mcp/{name}/tools` with `{"enabled": ["tool_a", …]}` sets the
grant; unknown tool names are rejected. `GET /api/config/mcp` lists each
server's discovered tools with their `enabled` flag, its trust, and whether
it is connected. Ordinary chats receive only the enabled tools of connected
servers; nothing is appended "because it is connected".

### Secret boundaries

Each server's `headers` are sent only on that server's own connection; they
are never returned by the config API and never shared with another server,
a skill, or the model. The env-secret redaction list also covers common key
names in tool output.

### Side effects

MCP tools run inside the ordinary chat tool loop with the same approvals and
logging as built-in tools, and they are never part of proactive routines
(see `docs/proactive-loop.md`).

## MCP Apps

Some tools ship a small HTML UI (MCP Apps). Nolune renders it only for
`curated` servers whose tool is enabled, through the host bridge in
`McpAppViewer.svelte`.

### Sandbox and threat model

- The HTML is loaded with `srcdoc` into an `<iframe sandbox="allow-scripts">`.
  The frame has an opaque origin: no access to Nolune's origin, cookies,
  storage, or the parent document; no `allow-same-origin`, `allow-popups`,
  `allow-forms`, or `allow-top-navigation`, so the content cannot navigate the
  page, open windows, or submit forms on its own.
- The only channel is `postMessage` through the MCP Apps bridge. The host
  honours size changes, inline/fullscreen display mode, tool input/result
  delivery, and `openLink` for `http`/`https` URLs only, opened with
  `noopener,noreferrer`.
- Capability URLs for media are short-lived and scoped (see
  `docs/security/resource-url-inventory.md`); the frame never receives a
  control token.
- Untrusted (custom) servers never get an app frame, even if their tools are
  enabled; their tools still run text-only.

Remaining risk: an app can render misleading content inside its own frame.
It cannot read or change anything outside it.
