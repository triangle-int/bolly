<script lang="ts">
	import { Dialog } from "bits-ui";
	import { getViewerFile, closeFile } from "$lib/stores/fileviewer.svelte.js";

	const file = $derived(getViewerFile());

	async function download() {
		if (!file) return;
		try {
			const res = await fetch(file.url);
			if (!res.ok) throw new Error("Download failed");
			const blob = await res.blob();
			const blobUrl = URL.createObjectURL(blob);
			const a = document.createElement("a");
			a.href = blobUrl;
			a.download = file.name;
			document.body.appendChild(a);
			a.click();
			document.body.removeChild(a);
			setTimeout(() => URL.revokeObjectURL(blobUrl), 1000);
		} catch {
			window.open(file.url, "_blank");
		}
	}
</script>

{#if file}
<Dialog.Root open={true} onOpenChange={(open) => { if (!open) closeFile(); }}>
<Dialog.Portal><Dialog.Overlay class="fixed inset-0 z-[500] bg-background/95" />
<Dialog.Content class="fixed inset-0 z-[501] flex flex-col items-center justify-center gap-4 p-6">
<Dialog.Title class="sr-only">{file.name}</Dialog.Title>
<Dialog.Description class="sr-only">File preview. Download the file or close the preview.</Dialog.Description>
<div class="viewer-content">

			{#if file.type === "image"}
				<img src={file.url} alt={file.name} class="viewer-img" />
			{:else if file.type === "video"}
				<!-- svelte-ignore a11y_media_has_caption -->
				<video src={file.url} controls autoplay class="viewer-media"></video>
			{:else if file.type === "audio"}
				<div class="viewer-audio-wrap">
					<div class="viewer-icon">
						<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M9 18V5l12-2v13" stroke-linecap="round" stroke-linejoin="round"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>
					</div>
					<span class="viewer-label">{file.name}</span>
					<!-- svelte-ignore a11y_media_has_caption -->
					<audio src={file.url} controls autoplay class="viewer-audio"></audio>
				</div>
			{:else if file.type === "pdf"}
				<iframe src={file.url} title={file.name} class="viewer-pdf"></iframe>
			{:else}
				<div class="viewer-file-wrap">
					<div class="viewer-icon">
						<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
							<path d="M6 2h9l5 5v13a2 2 0 01-2 2H6a2 2 0 01-2-2V4a2 2 0 012-2z" stroke-linejoin="round"/>
							<path d="M14 2v6h6" stroke-linejoin="round"/>
						</svg>
					</div>
					<span class="viewer-label">{file.name}</span>
				</div>
			{/if}
		</div>

		<div class="viewer-toolbar">
			<span class="viewer-name">{file.name}</span>
			<div class="viewer-actions">
				<button class="viewer-btn" onclick={download} title="Download" aria-label="Download file">
					<svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M10 3v10m0 0l-3.5-3.5M10 13l3.5-3.5M4 15.5v1h12v-1"/></svg>
				</button>
				<button class="viewer-btn" onclick={closeFile} title="Close" aria-label="Close preview">
					<svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M5 5l10 10M15 5L5 15"/></svg>
				</button>
			</div>
		</div>
</Dialog.Content></Dialog.Portal></Dialog.Root>
{/if}

<style>
.viewer-content{max-width:90vw;max-height:calc(100dvh - 130px);display:flex;align-items:center;justify-content:center}.viewer-img,.viewer-media{max-width:90vw;max-height:calc(100dvh - 130px);object-fit:contain;border-radius:8px}.viewer-audio-wrap,.viewer-file-wrap{display:flex;flex-direction:column;align-items:center;gap:16px;padding:32px;border:1px solid var(--border);border-radius:16px;background:var(--card)}.viewer-icon{width:48px;height:48px;color:var(--primary)}.viewer-icon svg{width:100%;height:100%}.viewer-label{font-size:16px;color:var(--foreground);overflow-wrap:anywhere}.viewer-audio{width:320px;max-width:75vw}.viewer-pdf{width:85vw;height:calc(100dvh - 150px);border:none;border-radius:8px;background:white}.viewer-toolbar{position:fixed;bottom:0;left:0;right:0;display:flex;align-items:center;justify-content:space-between;gap:16px;padding:12px 20px;padding-bottom:calc(12px + env(safe-area-inset-bottom,0px));background:var(--card);border-top:1px solid var(--border)}.viewer-name{font-size:14px;color:var(--text-secondary);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0}.viewer-actions{display:flex;gap:8px;flex-shrink:0}.viewer-btn{display:grid;place-items:center;width:44px;height:44px;border:1px solid var(--border);border-radius:8px;background:var(--secondary);color:var(--foreground);cursor:pointer}.viewer-btn:hover{background:var(--accent)}.viewer-btn svg{width:18px;height:18px}
</style>
