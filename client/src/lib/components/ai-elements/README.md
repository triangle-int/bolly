# Svelte AI Elements for Nolune

Selected source files imported from https://svelte-ai-elements.vercel.app/r/{prompt-input,message,conversation,tool}.json on 2026-09-19. Upstream: https://github.com/SikandarJODD/ai-elements (MIT; see LICENSE).

This is a deliberately selected source installation, not the full registry. Existing shadcn-svelte primitives, Bits UI, and Lucide are reused. Runtime utility: runed. No AI SDK backend is required.

Local adaptations:
- Resolved registry import aliases to `$lib`.
- Prompt submission carries raw attachment Files to the existing upload API; removed AI SDK types and eager base64 conversion.
- Added disabled and in-flight submit guards; false return retains drafts. Textarea forwards native accessibility/disabled attributes, and Submit marks itself for Enter-key dispatch.
- ConversationContent fixes the registry's duplicate element binding and supports `autoScroll={false}` for the existing application's scroll policy.
- Tool uses installed Bits UI directly, stable Svelte IDs, token colors, and a neutral Recorded state for activity without authoritative lifecycle metadata.

App adapters and brand styling live in `../chat/`; see `docs/design-system.md` and the `/design-system` client route. Avoid blindly overwriting these files with a registry update; compare behavior and reapply the documented adaptations.
