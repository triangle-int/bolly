# Nolune · Little Moon design system

Use this guide before changing the client, landing, onboarding, or brand assets. The approved direction is **Little Moon**. The product name is Nolune (wordmark: `nolune`).

## Source of truth

- Client color tokens: `client/src/lib/styles/tokens.css`.
- Shared application CSS primitives: `client/src/routes/layout.css` (`nl-button`, `nl-button-secondary`, `nl-panel`, `nl-input`, `nl-eyebrow`).
- Interactive examples: client route `/design-system`, including the real message, composer, and tool components. Examples are explicitly sample content and do not require a backend.
- Landing: `landing/src/app.css` and `landing/src/lib/components/`.
- Desktop (Tauri): `desktop/src/app.css` mirrors the client tokens and `nl-*` primitives; fonts are bundled from Fontsource because the app CSP only allows same-origin assets. Desktop avatar: `desktop/src/lib/components/Moon.svelte`.
- Avatar: `client/static/skins/moon/character.svg`; thinking expression beside it. Landing copy: `landing/static/assets/nolune-moon.svg`. Desktop copy: the inline `Moon.svelte` component and the app icon source generated from the same path.
- Animated marketing avatar: `landing/src/lib/components/MoonCompanion.svelte`.

Update the guide and reference page when intentionally changing this system. Reuse existing components and tokens before inventing another variant. Do not automatically create a new palette for each feature.

## Character and tone

Calm, capable, personal. Generous space, simple forms, and readable text. Use flat plum surfaces, lavender actions, and ivory type. The landing may use ivory and pale-lavender section backgrounds. The working client uses a consistent dark plum theme.

Little Moon is the only skin. Its persisted identifier is `moon`. Minty and the golden liquid orb are retired. Do not restore glass-orb imagery, blue-black surfaces, glowing buttons, grain overlays, or translucent message bubbles.

Write clear sentence-case labels: “Create your companion”, “Retry connection”. Never present disconnected or failed data loading as an empty account, a successful action, or a live example.

## Color roles

| Role | CSS token | Value |
| --- | --- | --- |
| App background | `--background` | `#201D29` |
| Panel | `--card` | `#282432` |
| Elevated surface | `--popover` | `#312C3C` |
| Main text | `--foreground` | `#F6F3EC` |
| Secondary text | `--text-secondary` | `#D0C9DC` |
| Muted text | `--text-muted` | `#B5ADC2` |
| Primary action / focus | `--primary`, `--ring` | `#B7A9E7` |
| Text on lavender | `--primary-foreground` | `#201D29` |
| Selected surface | `--accent` | `#3B334D` |
| Border | `--border` | `#51485F` |
| Input border | `--input` | `#625770` |
| Destructive text | `--destructive` | `#F39B9B` |

Use semantic tokens rather than literal hex values in client components. `--warm` and `--glass-*` are existing component aliases mapped to the new system; they do not authorize amber colors or glass effects. Status colors must be accompanied by text, never used as the sole signal. Check text contrast against the actual background; do not dim essential labels with extra opacity.

## Typography

- **Fraunces 400**: welcome headings and editorial moments. Use upright type; italic only as a deliberate short emphasis. Heading line height 1.1–1.2, tracking no tighter than -0.035em.
- **Bricolage Grotesque**: navigation, controls, body, messages, settings. Body 16px, compact body 14px, helper text 12–13px. Line height 1.5–1.7.
- **JetBrains Mono**: code, commands, technical identifiers; never the default navigation font.
- Heading sizes: 40–72px for welcoming screens, 28–36px for sections, 18px for card titles. Reduce welcome headings on phones.

## Spacing and shape

Use the 4px spacing scale: 4, 8, 12, 16, 24, 32, 48. Typical panel padding 24px, page gutter 40px desktop / 20px mobile. Controls use 8px radius, panels 16px, chat inputs 12px. Use thin solid borders and minimal shadows. Avoid nested decorative cards.

## Component rules

- Primary action: lavender fill, plum text, minimum 44px height, 14px medium sans text.
- Secondary action: panel fill with border and ivory text. Preserve disabled and pending states.
- Selects: use the shared shadcn-svelte `Select` components, with a 44px trigger and option targets. Menus use popover tokens and visible selected/focus states. The composer model selector is the reference.
- Inputs: visible label, 16px text, solid panel surface, focus outline. Placeholder text is not a label.
- Chat: lavender-tinted user bubble, plum companion bubble. Keep text selection, file links, streaming, and tool interactions intact.
- Navigation: clear active surface and lavender indicator; scroll horizontally when space is limited rather than hiding destinations.
- Empty states: one clear next step and brief explanation. Keep loading, disconnected, authenticated-empty, and populated states distinct.
- Icons: existing Lucide components or simple outlined SVGs, 1.5–2px stroke. Do not substitute ambiguous Unicode glyphs for document/action icons.

## Avatar and motion

Use the approved lavender crescent path and small dark eyes. Keep the face minimal. Use the thinking SVG for actual thinking state, never an unrelated randomized video.

On the landing, the moon can float, blink, glance, and change expression on activation. Keep motion gentle and avoid changing animation duration on hover (this shifts phase and visibly snaps). Supply pause for continuous decorative motion and honor `prefers-reduced-motion`. Essential UI must remain usable without animation. Do not hide the avatar in reduced-motion mode.

## Accessibility and responsive behavior

Visible keyboard focus; semantic buttons/links; meaningful labels for icon-only controls; decorative SVGs hidden from assistive technology. Main controls have 44px targets. Do not disable browser zoom. Support 390px mobile widths without page overflow. Long commands can scroll inside their own container. Announce asynchronous errors and success without stealing focus.

## Verification for future changes

1. Run `corepack pnpm@10.34.5 --dir client check` and `build` (likewise `landing` when affected).
2. Inspect affected screens on desktop and phone widths, including focus and disabled/error states.
3. Use `/design-system` to compare tokens, controls, and actual message rendering.
4. Verify live behavior with a running server; when unavailable, explicitly report that limitation. Never substitute mock data without labeling it.
5. Keep Minty removed. No existing installations required migration at the time of this redesign.

## Svelte AI Elements

The client vendors a focused subset from [Svelte AI Elements](https://svelte-ai-elements.vercel.app/docs/installation) under `client/src/lib/components/ai-elements/`. Upstream license and integration notes live alongside the source. These are editable source components, not a separate chat backend.

- `MessageBubble` composes Message and MessageContent, sanitizes assistant markdown, preserves file viewing and word-level voice reveal, and renders unfinished streams as plain text.
- `PromptComposer` composes Prompt Input, Textarea, Toolbar, and Submit. Reuse it for new conversation surfaces. It has no API dependencies; `ChatInput` supplies model preferences, upload progress, and the real send callback.
- `ChatView` uses Conversation/Content with `autoScroll={false}` because its existing WebSocket and voice code manages scrolling. Do not enable two competing scroll controllers.
- `StreamActivity` uses Tool/ToolHeader/ToolContent. Historical activity is labeled “Recorded”, not “Running” or “Completed”: the current activity record does not prove execution status.
- Preserve raw `File` uploads through Nolune's API. Do not introduce base64 encoding or an AI SDK backend just to use presentation components.
- A send callback can return `false` or reject to preserve the draft and files. Disable duplicate submissions; keep the Stop button available while the agent runs. Enter sends, Shift+Enter inserts a line, IME composition must not submit.
- Add Sources or Confirmation only when the backend supplies real citations or approval state. Never invent source links, execution status, or approval outcomes.

The `/design-system` conversation example explicitly simulates streaming, stop, and failure states without a backend. Its attachments are not uploaded. Use it to verify changes, then exercise real chat against a running server before claiming end-to-end validation.

## Application surfaces

The same tokens apply to settings, onboarding, authentication, agent management, skills, drops, thoughts, memory, statistics, file previews, and notification surfaces. Keep collection errors distinct from empty results; zero-activity statistics must not invent a peak hour or day.

- Shared shadcn Button, Input, and Select triggers default to at least 44px targets. Inputs keep 16px type on phones.
- Destructive companion confirmation uses shadcn AlertDialog. File previews and secret entry use Bits UI Dialog; keep focus trapping, Escape, and focus restoration intact. New-skill entry uses native dialog semantics.
- Navigation has `aria-current`, horizontal scrolling on phones, and a lavender active indicator.
- The working moon is centered within its scene element and capped at 200px on phones so it does not clip or compete with the composer.
- Conversation history failures remain visible with Retry and Settings actions. Provider setup is required for live chat; the design-system example stays explicitly local and simulated.

Onboarding uses `MoonBirth.svelte` after skin selection: the crescent gently grows into place, opens its eyes, blinks, and greets the user. It completes automatically, has a Continue control, and shows a still moon with a shorter hold for reduced motion. Replay the same component in `/design-system`.

## Desktop app

The Tauri desktop app (`desktop/`) is a connection shell, not a second chat client. It uses the same plum surfaces, lavender actions, Fraunces headings, and `nl-button`, `nl-button-secondary`, `nl-panel`, `nl-input`, `nl-eyebrow` primitives as the client.

- Splash: the crescent grows into place and opens its eyes (same keyframes as `MoonBirth.svelte`), then the `nolune` wordmark fades in. It has a Continue control and shows a still moon with a shorter hold for reduced motion. The golden orb video and its spark sound are retired; the splash plays a short CC0 chime (`desktop/static/splash.mp3`, from videoeditingsfx.com, no attribution required) at 0.4 gain, timed to the moon opening its eyes.
- Connect panel: visible labels, 16px inputs, one primary action (Save connection or Open companion) and secondary actions beside it. Errors use `--destructive` text with `role="alert"`; pending state shows a spinner with text.
- Settings: permissions are a bordered list on `--card`; a granted state is a text badge, an ungranted state is a lavender Grant button. Failed status checks show an error and Retry, never an empty list.
- Overlay: the working moon sits in a small card-colored disc with gentle float and blink; action flashes are card-colored chips with outlined SVG icons and JetBrains Mono text. No emoji glyphs.
- App icon: lavender crescent on a plum rounded square, regenerated with `pnpm tauri icon` from a 1024px PNG of the approved path.
