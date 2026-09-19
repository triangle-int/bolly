# Project conventions

## Design system

Before changing client or landing UI, read [docs/design-system.md](docs/design-system.md). Use the Little Moon tokens and shared UI patterns; keep the `/design-system` reference page in sync. Little Moon is the only skin; do not reintroduce Minty or the golden orb.


## Versioning

Single source of truth: `VERSION` file in repo root.

To bump version across all packages (server, client, landing, desktop):
```sh
./scripts/bump-version.sh 0.14.0
```

Or edit `VERSION` and run without args to sync:
```sh
./scripts/bump-version.sh
```

After bumping, commit all changed files, tag, and push:
```sh
git add -A
git commit -m "Bump version to vX.Y.Z"
git tag vX.Y.Z
git push && git push origin vX.Y.Z
```

## Package manager

Use `pnpm` (not npm) for client and landing.

## Migrations

Never write migrations manually — always use `drizzle-kit generate`.

## Avatar assets

Little Moon uses SVG assets in `client/static/skins/moon/`. Keep the desktop overlay and landing copies consistent. Test expressions in `/design-system`; no video skin pipeline is needed.

## Uploading local files to fal.ai

The fal.ai MCP `upload_file` tool doesn't support local file paths over HTTP. Use an ngrok tunnel:

```sh
# 1. Start a temporary HTTP server in the directory with the file
python3 -m http.server 18923 --directory /path/to/dir &

# 2. Expose it via ngrok
ngrok http 18923 --log=stdout > /tmp/ngrok-fal.log 2>&1 &
sleep 3

# 3. Get the public URL
curl -s http://127.0.0.1:4040/api/tunnels | python3 -c "import sys,json; print(json.load(sys.stdin)['tunnels'][0]['public_url'])"

# 4. Use the URL with fal.ai upload_file tool: {ngrok_url}/filename.png
# 5. Kill both processes when done
```
