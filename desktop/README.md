# Bolly desktop

Enter the **root** HTTP(S) origin of your own Bolly server and its auth token.
Localhost, IPv4, bracketed IPv6, hostnames, and explicit ports are supported.
Base paths such as `/bolly` are rejected because the client uses root-relative
routes. **Test connection** checks authenticated metadata without saving or
enabling computer use. **Save connection** stores the connection, and **Open /
Reconnect** opens the companion and starts the existing machine WebSocket bridge.

## Credentials and authentication

Tokens live in the OS credential store, through the maintained
[Keyring 4 API](https://docs.rs/keyring/4.2.0/keyring/v1/index.html): macOS Keychain
Services, Windows Credential Manager, or Linux Secret Service (for example GNOME
Keyring/KWallet). Linux requires an available, unlocked Secret Service session;
there is no plaintext fallback. Flatpak includes permission to reach that service.
`settings.json` contains only the server URL, an opaque random secret reference,
and opaque cleanup references for recovering interrupted saves. Recording
preferences remain separate. Editing with a blank token keeps the saved secret.

Legacy `session` and plaintext `self_hosted` settings are removed before migration
is attempted. A valid old self-hosted connection is migrated to the OS store;
invalid records or an unavailable keychain require re-entry. Interrupted writes
and superseded secrets are cleaned up through the reference journal. Failed
cleanup remains retryable rather than silently dropping the last reference.

The server's existing auth middleware accepts Bearer headers and `bolly_token`
cookies. However, Tauri 2.10's `set_cookie(Cookie)` API has no source URL or portable
host-only flag: Wry's WebKit, WebView2, and WebKitGTK adapters construct cookies
from a domain. Moreover, even host-only cookies cannot isolate ports or HTTP from
HTTPS. Therefore **the desktop never sets a server-token cookie**.

Instead, a native relay listens on a random IPv4 loopback port and forwards to
exactly the configured upstream scheme, host and port. Only native requests get
the server Bearer token; redirects are rejected, browser cookies are not forwarded,
and upstream `Set-Cookie` headers are stripped. Metadata validation, keychain
reads, machine connection and relay configuration run in Rust. Saved reconnect
sends only an opaque reference over IPC. Blank-token edits send the new origin
and existing reference; no
command returns a saved token. The frontend HTTP plugin is removed, a restrictive
CSP limits dashboard networking to IPC, and legacy `.cookies` caches are removed.
HTTP uploads and binary media are streamed; textual responses are buffered and
sanitized before release,
and the UI WebSocket is relayed with header authentication. The machine WebSocket
continues to connect directly to `/api/agents/ws/machine` with its Bearer header.

The companion runs in a separate incognito webview without native capabilities.
Executable HTML and JavaScript from the validated, self-hosted upstream are part
of the trusted application boundary; arbitrary JavaScript is not treated as
untrusted data. Dynamic text is sanitized separately. A relay-owned CSP confines
fetch/WebSocket, images, media, forms, frames and other resource loads to the
exact ephemeral relay, and the companion denies every external navigation and
new-window request rather than forwarding it to the system browser.
An origin-checked initialization script bootstraps a random, process-lifetime
relay session through a POST header, never a URL. The relay's separate session
cookie is host-only, HttpOnly and SameSite=Strict; it is **not the server token**.
Since cookies alone cannot restrict ports, the relay also verifies Host, Origin,
Referer and fetch metadata, rejecting sibling ports, hosts and cross-origin use.
The UI never reads this cookie or mirrors it into localStorage. Public media links
are translated to existing header-authenticated API routes, and credentials are
removed from embedded prose, Markdown, nested JSON and WebSocket output before
they reach the UI. Exact raw and percent-encoded secrets are also redacted when
a URL cannot be localized. Public media links carrying the configured token map
to the fixed upstream API even when generated using a different public origin.

**View → Back to Dashboard** keeps the machine bridge active. **Disconnect**
cancels the session, kills active subprocess groups/network operations, drains
dispatched main-thread and blocking actions, joins screen capture, revokes the relay, terminates its sockets and
streams, destroys the incognito companion, clears the old persistent browser
profile (including remote-origin localStorage, cookies, caches and service
workers), and removes keychain/settings credentials. Clearing the old profile is
also attempted at startup. Browser data removal uses the native webview API;
WebKit schedules its removal asynchronously. Errors remain visible and Disconnect
can be retried. No cloud sign-in or custom-scheme deep links are registered.

## Development checks

```sh
cd desktop
pnpm test
pnpm check
pnpm build
cd ..
cargo +1.93.1 check
cargo +1.93.1 test
```

Tests exercise the real frontend state module with mocked keychain/settings
failures, migration and interrupted saves. Native tests run a real loopback HTTP
and WebSocket server to check origin boundaries, authentication, media forwarding,
redirect rejection and session revocation. They do not access the developer's
keychain. Native GUI/keychain behavior on all three operating systems still needs
platform smoke testing with a running Bolly server and computer-use permissions.
