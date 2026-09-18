# Nolune launch cutover

The product is Nolune and its domain is `nolune.dev`. There are no existing
installations, so this rename intentionally provides no Bolly configuration
aliases, data migration, or desktop upgrade compatibility.

## Release prerequisites

- Rename the GitHub repository from `triangle-int/bolly` to `triangle-int/nolune`
  before publishing the first Nolune release. The installer, server update checks,
  desktop updater, and landing script endpoints target the new repository.
  Update local Git remotes and any hosting integrations after the rename.
- Configure `nolune.dev` in the hosting provider, point DNS at that deployment,
  and verify TLS. Configure `docs.nolune.dev` and any hosted-instance subdomains
  only where corresponding services are actually deployed.
- Provision `support@nolune.dev` and `security@nolune.dev` before publishing the
  contact links. Update OAuth authorized origins/callbacks and deployment
  configuration, including `NOLUNE_PUBLIC_URL`, to match the actual services.
- If retaining the old domain, configure redirects from `bollyai.dev` to the
  corresponding new URLs after verifying the new deployment.
- Review signing/provisioning settings for desktop identifier
  `com.triangle-int.nolune-desktop`. Keep the existing updater signing key unless
  it is intentionally rotated together with the release signing configuration.

## Verification

- Build server, client, landing, and desktop. Run installer and release shell
  tests, desktop JavaScript tests, and Rust tests.
- Publish a versioned release with `nolune-server-*` assets and Nolune desktop
  bundles. Verify `latest.json` and the desktop updater signatures.
- Verify `/install.sh` and `/uninstall.sh` on `nolune.dev` serve the renamed scripts.
- Smoke-test fresh macOS/Linux installs, service startup, configuration at
  `~/.nolune`, authentication, updates, and uninstall. Verify desktop connections
  and credential storage on each supported desktop platform.
- The Flatpak manifest remains a packaging template: set its release asset URL
  and SHA-256 to a real Nolune Debian artifact before distributing it.

## External skills registry

The separate `triangle-int/bolly-skills` repository retains its current URLs.
Renaming that repository is a separate operation; changing its URLs here before
it is moved would break skill discovery.

This checklist records external work; the code rename does not configure DNS,
mailboxes, hosting, OAuth providers, or GitHub repository settings.
