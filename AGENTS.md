# Project conventions

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

## Transparent video (skin clips)

Skin clips need transparent video in two formats:
- **WebM** (VP9 alpha) — Chrome/Firefox
- **MOV** (HEVC alpha) — Safari

### Pipeline

1. **Remove background** via Bria video background removal on fal.ai:
   - Use `mov_proresks` output (ProRes with alpha) — webm_vp9 from Bria loses alpha
   - `background_color: "Transparent"`

2. **ProRes → WebM** (VP9 alpha):
   ```sh
   ffmpeg -i prores-alpha.mov -c:v libvpx-vp9 -pix_fmt yuva420p output.webm
   ```

3. **WebM → MOV** (HEVC alpha via macOS VideoToolbox):
   ```sh
   ffmpeg -c:v libvpx-vp9 -i input.webm -c:v hevc_videotoolbox -alpha_quality 0.75 -vtag hvc1 output.mov
   ```

### Important
- `ffmpeg -c:v hevc_videotoolbox` with `-vtag hvc1` is the **only** way to get HEVC alpha that Safari plays correctly
- `avconvert --preset PresetHEVCHighestQualityWithAlpha` does NOT produce working alpha
- Finder "Encode Selected Video Files" with "Preserve Transparency" also does NOT work
- `ffprobe` shows `yuv420p` for all VP9 alpha webm files — this is misleading, alpha is there
- Skin files go in `client/static/skins/{skin_name}/`
- Test page: `client/static/video-test.html`

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
