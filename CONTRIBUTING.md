# Contributing to Nolune

Thanks for your interest in contributing! Here's how to get started.

## Development Setup

### Prerequisites

- **Rust** (latest stable)
- **Node.js** (LTS) + **pnpm**
- **PostgreSQL** (for landing page, via Neon or local)

### Server

```bash
cd server
cp config.example.toml config.toml
# Edit config.toml with your API keys
cargo run
```

### Client

```bash
cd client
pnpm install
pnpm dev
```

### Landing

```bash
cd landing
cp .env.example .env
# Fill in environment variables
pnpm install
pnpm dev
```

### Desktop (Tauri)

```bash
cd desktop
pnpm install
pnpm tauri dev
```

## Pull Requests

1. Fork the repo and create your branch from `main`
2. Make your changes
3. Test your changes locally
4. Create a pull request with a clear description

## Versioning

Single source of truth: `VERSION` file in repo root.

```bash
./scripts/bump-version.sh 0.20.0
```

## Code Style

- **Rust**: `cargo fmt` + `cargo clippy`
- **TypeScript/Svelte**: follow existing patterns
- **Commits**: short imperative descriptions

## Reporting Issues

Use [GitHub Issues](https://github.com/triangle-int/nolune/issues). Include:
- Steps to reproduce
- Expected vs actual behavior
- Server logs if applicable

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
