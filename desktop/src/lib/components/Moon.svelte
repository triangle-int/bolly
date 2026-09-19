<script lang="ts">
  /** Little Moon avatar. Same path as client/static/skins/moon/character.svg. */
  let {
    size = 32,
    label = "",
    class: className = "",
  }: { size?: number | string; label?: string; class?: string } = $props();
</script>

<svg
  class={className}
  viewBox="0 0 320 320"
  fill="none"
  width={size}
  height={size}
  role={label ? "img" : undefined}
  aria-label={label || undefined}
  aria-hidden={label ? undefined : "true"}
>
  <path
    class="moon-body"
    d="M155 24C161 23 164 29 160 34C127 74 130 129 157 168C183 207 228 224 277 210C284 208 289 214 285 221C262 266 217 292 170 290C93 287 36 230 36 157C36 91 85 34 155 24Z"
    fill="var(--primary)"
  />
  <g class="moon-eyes" fill="var(--primary-foreground)">
    <ellipse cx="81" cy="164" rx="6" ry="9" />
    <ellipse cx="110" cy="164" rx="6" ry="9" />
  </g>
</svg>

<style>
  svg {
    display: block;
    flex-shrink: 0;
    overflow: visible;
  }
</style>
