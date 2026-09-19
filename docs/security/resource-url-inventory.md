# Resource URL migration (#116)

Scope: HTTP resource URLs. `/api/ws` query authentication is tracked separately in #112.

| Surface | Producers / consumers | Required contract |
| --- | --- | --- |
| Server attachments | `llm/helpers.rs`, `tools/files.rs` | Exact upload capability; model-provider audience; reusable bounded lifetime |
| Server memory | `tools/memory_tools.rs`, `tools/mod.rs`, `routes/instances.rs` | Root and nested memory paths; canonical component encoding |
| Prompts | `chat.rs`, `child_agents.rs` | No control-token URL templates or secrets |
| Media/provider tools | `tools/media.rs`, `tools/computer.rs` | Scoped URLs for audio, video, images and documents |
| Browser uploads | `api/client.ts`, `MessageBubble.svelte`, file previews | Authenticated issuance, refresh before expiry |
| Browser memory | `MemoryMapView.svelte`, memory search | Authenticated exact-resource issuance |
| Export | `api/client.ts`, settings | Bearer fetch, downloaded blob |
| Native relay | `companion_relay.rs` | Fixed upstream, native-relay audience, no forwarded control-token query |
| Upload API | `routes/uploads.rs`, desktop upload bridge | Bearer headers; resource verifier is separate |
| Resource handlers | upload and memory readers | Verify method/audience/instance/resource/canonical URI before filesystem access |
| Rotation | `app/state.rs`, `routes/config.rs` | Replace capability service; invalidate old grants; empty-auth issues no grant |

## Implemented contract

- `AppState.resources` shares a replaceable `CapabilityService` with every producer. Each startup/reload/token update creates a fresh generation key; existing producer handles use that generation. Empty authentication stores no service and produces query-free resource URLs.
- `/resources/browser/{files|memory}/…`, `/resources/model-provider/{files|memory}/…`, and `/resources/native-relay/{files|memory}/…` have separate handlers with fixed audiences. They check GET, exact canonical URI encoding, instance and resource identity, expiry, and replay before calling the file readers. Resource responses use `private, no-store`.
- Browser issuance is authenticated POST to `/api/instances/{slug}/resource-capabilities/{files|memory}`. Native issuance uses `/api/native-relay/instances/{slug}/resource-capabilities/{files|memory}`. Inputs are one `id` or one `path`, bounded by a 2 KiB body limit and the primitive's identity limits; unknown fields (including audience) are rejected.
- Model-provider grants are reusable for at most 15 minutes. Browsers renew after 10 minutes, retry failed issuance, and renew persisted links on load. Provider attachment history is renewed for the configured origin and current instance before a new turn.
- The native relay exchanges exact resource identities for NativeRelay grants using its native Bearer credential; the resource GET carries only the scoped capability.
- Legacy `/public/files` and `/public/memory` server routes deny access. Client saved-link handling converts legacy public/API resource identities to fresh browser grants. Authenticated API readers continue to require Bearer headers. Export uses authenticated fetch and a blob.
- The only remaining query-control-token producer is the explicitly marked `/api/ws` handshake (#112). Recursive scans exempt only that function/block, preserving production code after inline test modules.
- Control tokens are registered for redaction at configuration time and rotation. Tool results/errors/activity, provider payloads/errors, and server log formatting redact them.

## Verification

Focused red-to-green tests were run for issuance, audience/canonical/method checks, producers, legacy rejection, reload/rotation, secret redaction, browser issuance/export/refresh, native exchange, cache policy, media MIME types, saved attachments, and memory API encoding. Additional tests cover root/nested paths, Unicode/NFC, exact percent encoding, retry/replay, expiry boundaries, empty authentication, and bounded inputs.

Checks: `cargo test -p server`, `cargo test -p nolune-desktop`, client and desktop `pnpm test`, `pnpm check`, and `pnpm build`; installer/release shell suites and the passive-capture regression script. The live browser smoke check uses a temporary workspace and verifies a Unicode memory image, an uploaded chat attachment, a historical memory link, and the file viewer with no HTTP control-token queries. No model generation or real provider credentials are needed for that fixture.
