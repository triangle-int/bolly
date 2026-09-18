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

## CI checks

GitHub Actions runs `.github/workflows/ci.yml` on pull requests, pushes to
`main`, and manual dispatches. It checks Rust formatting, tests and Clippy,
all three Svelte frontends, desktop JavaScript tests,
and the installer/release shell regression tests. Rust desktop checks run on
macOS; server checks run on Linux. CI uses the Rust version in
`rust-toolchain.toml`, Node.js 22, and pnpm 10.34.5.

To run the checks locally:

```bash
cargo fmt --all -- --check
for project in client desktop landing; do
  pnpm --dir "$project" install --frozen-lockfile
  pnpm --dir "$project" check
  pnpm --dir "$project" build
done
pnpm --dir desktop test
# Build the frontends above before checking Rust (the server embeds client/build).
# Native dependencies are the same as for development of each package.
cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets
bash scripts/tests/install.sh
bash scripts/tests/release-workflow.sh
```

## Reporting Issues

Use [GitHub Issues](https://github.com/triangle-int/nolune/issues). Include:
- Steps to reproduce
- Expected vs actual behavior
- Server logs if applicable

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
