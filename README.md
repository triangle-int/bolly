<p align="center">
  <img src="landing/static/assets/nolune-moon.svg" alt="Nolune, a lavender crescent companion" width="200" />
</p>

<h1 align="center">Nolune</h1>

<p align="center">
  <strong>Your AI, with a computer of its own.</strong><br>
  A personal AI companion that lives on your machine and works across your computers.
</p>

<p align="center">
  <a href="https://nolune.dev">Website</a> &nbsp;&bull;&nbsp;
  <a href="https://github.com/triangle-int/nolune/releases">Download</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/rust-2024-CE422B?style=flat-square&logo=rust" alt="Rust" />
  <img src="https://img.shields.io/badge/sveltekit-5-FF3E00?style=flat-square&logo=svelte" alt="SvelteKit" />
  <img src="https://img.shields.io/badge/tauri-2-24C8D8?style=flat-square&logo=tauri" alt="Tauri" />
  <img src="https://img.shields.io/badge/license-MIT-A97CF8?style=flat-square" alt="MIT License" />
</p>

<br>

Nolune is an open-source, self-hosted AI companion with a persistent identity, long-term memory, and the ability to use real computers.

Run Nolune's always-on brain on a Mac mini, home server, or spare computer. Then connect the desktop app on your other machines so Nolune can work with their screens, apps, files, and terminals from one interface.

Unlike a faceless agent dashboard, Nolune has a voice, personality, mood, and animated presence you can make your own. Your companion and its memories stay on hardware you control.

<br>

## How it works

### 1. Give Nolune a home

Install the Nolune server on an always-on macOS or Linux machine. It keeps your companion running, stores its memory, and coordinates its work.

```bash
curl -fsSL https://nolune.dev/install.sh | bash
```

Open `http://localhost:26559`. The first browser has to be paired: run `nolune pair` on the machine you just installed on and enter the eight-digit code it prints. Then follow the onboarding.

Paired browsers stay signed in. Review or revoke them, or mint a code for another device, under **Settings → Server**. The installer adds `~/.nolune/bin` to your `PATH`; until you open a new shell, use `~/.nolune/bin/nolune pair`.

### 2. Connect your computers

Install the desktop app on the computers where you want Nolune to act. Each connected machine becomes another place where your companion can see the screen, use apps, work with files, and run commands.

For example, Nolune can live on a Mac mini at home while you talk to it through the desktop app on your MacBook.

### 3. Ask it to do the work

Talk to Nolune normally. It can research something on the web, organize files, work inside an app, run a command on a connected machine, or continue a task while you are away.

Computer-use actions appear through a visible desktop overlay. You choose which machines to connect and grant the operating-system permissions they need.

<br>

## Why Nolune

### A companion, not a control panel

Nolune is designed around one persistent character rather than a collection of disposable chats. Its personality lives in `soul.md`, its mood changes over time, and its animated skin gives it a recognizable presence.

### One mind across multiple computers

The server holds Nolune's identity and memory. Connected desktop apps give it eyes and hands on other machines. You can keep the server somewhere reliable and interact with the same companion from wherever you work.

### Memory you can inspect

Nolune stores memories as ordinary files organized by topic. You can read them, edit them, back them up, or remove them. Keyword and semantic retrieval bring relevant memories back into later conversations.

### Proactive when you want it to be

Heartbeats and schedules let Nolune check in, follow recurring routines, and start useful work without waiting for a new message every time.

### Yours from end to end

Nolune is self-hosted and BYOK. There is no required Nolune cloud account, and Nolune does not impose its own usage limits. Your model provider's pricing and limits still apply.

<br>

## Your companion

<p align="center">
  <img src="client/static/skins/moon/character.svg" alt="Nolune Little Moon companion" width="200" />
</p>

**Nolune · Little Moon** — a curious lavender crescent. One familiar identity across your computers, with memories and personality that stay yours.

Our [design system](docs/design-system.md) documents the visual language, reusable UI patterns, and accessibility rules. Run the client and open `/design-system` for the interactive reference.

<br>

## Capabilities

- **Computer use** — See the screen, click, type, scroll, and use apps on connected machines.
- **Remote files and shell** — Read and edit files or run commands on the server and connected computers.
- **Persistent memory** — Remember people, preferences, projects, and shared moments across conversations.
- **Voice** — Speak through optional text-to-speech and voice mode.
- **Web and communication** — Search the web, work with email, and use installed integrations.
- **Skills and MCP** — Add new workflows and connect external tools without changing Nolune's core.
- **Schedules and heartbeats** — Run recurring routines and initiate conversations proactively.

<br>

## Install

### Server

The one-line installer sets up the native Nolune server and service on macOS or Linux:

```bash
curl -fsSL https://nolune.dev/install.sh | bash
```

When installation finishes, open `http://localhost:26559` and complete onboarding.

### Desktop app

Download the desktop app from [GitHub Releases](https://github.com/triangle-int/nolune/releases). Builds are available for macOS, Windows, and Linux.

Connect it to your self-hosted Nolune server to use the companion interface and enable computer use on that machine.

The release workflow runs on `v*` tags; manual runs must select a `v*` tag. It publishes server binaries and macOS, Windows, and Linux desktop artifacts. Nothing in this workflow provisions or updates running servers.

<br>

## Architecture

```text
server/     Rust + Axum — always-on brain, tools, memory, and embedded web client
client/     SvelteKit 5 — companion interface
desktop/    Tauri 2 — native client and computer-use bridge
landing/    SvelteKit — public website and documentation
```

| Layer | Technology |
|-------|------------|
| Server | Rust, Axum, Tokio |
| LLM | Anthropic or OpenAI API |
| Web client | SvelteKit 5, Tailwind CSS |
| Desktop | Tauri 2 |
| Memory | File-based storage with keyword and vector search |
| Extensions | Skills and Model Context Protocol |
| Distribution | Native binaries, launchd, and systemd |

### Data layout

Everything important is stored as files under `~/.nolune`:

```text
~/.nolune/
├── config.toml
├── browser_sessions.json    paired browsers (hashes only)
└── instances/
    └── {slug}/
        ├── soul.md              personality definition
        ├── heartbeat.md         proactive behavior
        ├── mood.json            emotional state
        ├── memory/              long-term memory library
        ├── drops/               autonomous creative artifacts
        ├── uploads/             user-uploaded files
        ├── skills/              installed skills
        └── chats/               conversation history
```

<br>

## Configuration

Most settings are available through the interface. Advanced configuration lives at `~/.nolune/config.toml`.

| Environment variable | Description |
|----------------------|-------------|
| `NOLUNE_HOME` | Data directory, defaults to `~/.nolune` |
| `NOLUNE_AUTH_TOKEN` | API token override. The token is for automation, the CLI and the desktop app; browsers pair for a revocable session instead and are unaffected when it changes |
| `NOLUNE_PUBLIC_URL` | Public URL for the server |
| `ANTHROPIC_API_KEY` | Anthropic API key override |
| `OPENAI_API_KEY` | OpenAI API key override |
| `RUST_LOG` | Logging level, defaults to `info` |

<br>

## Updates

For one-line installations, Nolune checks for updates automatically. Apply an update through Settings or run:

```bash
~/.nolune/bin/update
```

### Uninstall

```bash
curl -fsSL https://nolune.dev/uninstall.sh | bash
```

Keep your data while removing the server:

```bash
KEEP_DATA=1 curl -fsSL https://nolune.dev/uninstall.sh | bash
```

<br>

## Development

See the [Nolune launch cutover checklist](docs/nolune-cutover.md) for repository, domain, and release setup.

```bash
# Server
cd server && cargo run

# Web client
cd client && pnpm install && pnpm dev

# Desktop app
cd desktop && pnpm install && pnpm tauri dev

# Landing site
cd landing && pnpm install && pnpm dev
```

Use `pnpm`, not npm, for the JavaScript workspaces. See [CONTRIBUTING.md](CONTRIBUTING.md) for the complete development setup.

## Security

Computer use and remote access are powerful capabilities. Only connect machines you control, keep your authentication token private, and review the permissions granted to the desktop app.

See [SECURITY.md](SECURITY.md) to report a vulnerability.

## License

MIT — see [LICENSE](LICENSE).

<br>

<p align="center">
  <img src="client/src/lib/assets/favicon.svg" alt="Nolune" width="32" />
  <br><br>
  <sub>Built by <a href="https://triangleint.com">Triangle Interactive LLC</a></sub>
</p>
