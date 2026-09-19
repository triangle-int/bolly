<script lang="ts">
	import { onMount } from "svelte";
	import { createSkill } from "$lib/api/client.js";
	import type { Skill } from "$lib/api/types.js";

	let {
		onclose,
		oncreated,
	}: {
		onclose: () => void;
		oncreated: (skill: Skill) => void;
	} = $props();

	let dialogEl: HTMLDialogElement;
	onMount(() => { dialogEl.showModal(); });
	let name = $state("");
	let description = $state("");
	let instructions = $state("");
	let icon = $state("~");
	let saving = $state(false);
	let error = $state("");

	function closeOnBackdrop(event: MouseEvent) {
		if (event.target !== dialogEl) return;
		const rect = dialogEl.getBoundingClientRect();
		if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) dialogEl.close();
	}

	function toId(name: string): string {
		return name
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, "_")
			.replace(/^_|_$/g, "");
	}

	async function handleSubmit() {
		if (!name.trim()) return;

		saving = true;
		error = "";

		try {
			const skill: Skill = {
				id: toId(name),
				name: name.trim(),
				description: description.trim(),
				icon: icon.trim() || "~",
				builtin: false,
				enabled: true,
				instructions: instructions.trim(),
			};
			const created = await createSkill(skill);
			oncreated(created);
		} catch (e) {
			error = e instanceof Error ? e.message : "failed to create skill";
		} finally {
			saving = false;
		}
	}
</script>

<dialog bind:this={dialogEl} class="modal" aria-labelledby="create-skill-title" onclose={onclose} onclick={closeOnBackdrop} onkeydown={() => {}}>
		<div class="modal-header">
			<h2 id="create-skill-title" class="modal-title">New skill</h2>
			<button aria-label="Close new skill" class="modal-close" onclick={onclose}>&times;</button>
		</div>

		<form class="modal-form" onsubmit={(e) => { e.preventDefault(); handleSubmit(); }}>
			<div class="field-row">
				<div class="field field-icon">
					<label class="field-label" for="skill-icon">Icon</label>
					<input
						id="skill-icon"
						class="field-input"
						type="text"
						maxlength="2"
						bind:value={icon}
						placeholder="~"
					/>
				</div>
				<div class="field field-name">
					<label class="field-label" for="skill-name">Name</label>
					<input
						id="skill-name"
						class="field-input"
						type="text"
						bind:value={name}
						placeholder="e.g. Code Reviewer"
						required
					/>
				</div>
			</div>

			<div class="field">
				<label class="field-label" for="skill-desc">Description</label>
				<input
					id="skill-desc"
					class="field-input"
					type="text"
					bind:value={description}
					placeholder="what does this skill do?"
				/>
			</div>

			<div class="field">
				<label class="field-label" for="skill-instructions">
					Instructions
					<span class="field-hint">Prompt instructions used when active</span>
				</label>
				<textarea
					id="skill-instructions"
					class="field-textarea"
					bind:value={instructions}
					placeholder="you are skilled at..."
					rows="5"
				></textarea>
			</div>

			{#if error}
				<p class="modal-error" role="alert">{error}</p>
			{/if}

			<div class="modal-actions">
				<button type="button" class="btn-cancel" onclick={onclose}>
					Cancel
				</button>
				<button type="submit" class="btn-create" disabled={saving || !name.trim()}>
					{saving ? "Creating…" : "Create skill"}
				</button>
			</div>
		</form>
</dialog>

<style>

	@keyframes fade-in {
		from { opacity: 1; }
		to { opacity: 1; }
	}

	.modal {
		width: 90%;
		max-width: 480px;
		background: var(--card);
		border: 1px solid var(--border);
		border-radius: 1rem;
		padding: 1.5rem;
		animation: none;
	}

	@keyframes modal-enter {
		from {
			opacity: 1;
			transform: translateY(12px) scale(0.97);
		}
		to {
			opacity: 1;
			transform: translateY(0) scale(1);
		}
	}

	.modal-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 1.25rem;
	}

	.modal-title {
		font-family: var(--font-display);
		font-size: 0.95rem;
		font-weight: 500;
		color: var(--foreground);
	}

	.modal-close {
		font-size: 1.1rem;
		color: var(--text-secondary);
		background: none;
		border: none;
		cursor: pointer;
		padding: 0.25rem;
		line-height: 1;
	}

	.modal-close:hover {
		color: var(--text-secondary);
	}

	.modal-form {
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}

	.field-row {
		display: flex;
		gap: 0.75rem;
	}

	.field {
		display: flex;
		flex-direction: column;
		gap: 0.35rem;
	}

	.field-icon {
		width: 4rem;
		flex-shrink: 0;
	}

	.field-name {
		flex: 1;
	}

	.field-label {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.06em;
		text-transform: none;
	}

	.field-hint {
		text-transform: none;
		letter-spacing: normal;
		color: var(--text-secondary);
		margin-left: 0.5rem;
	}

	.field-input {
		font-family: var(--font-body);
		font-size: 0.78rem;
		color: var(--foreground);
		background: var(--card);
		border: 1px solid var(--border);
		border-radius: 0.5rem;
		padding: 0.5rem 0.625rem;
		outline: none;
		transition: border-color 0.2s ease;
	}

	.field-input:focus {
		border-color: var(--border);
	}

	.field-input::placeholder {
		color: var(--text-secondary);
	}

	.field-textarea {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--foreground);
		background: var(--card);
		border: 1px solid var(--border);
		border-radius: 0.5rem;
		padding: 0.5rem 0.625rem;
		outline: none;
		resize: vertical;
		line-height: 1.5;
		transition: border-color 0.2s ease;
	}

	.field-textarea:focus {
		border-color: var(--border);
	}

	.field-textarea::placeholder {
		color: var(--text-secondary);
	}

	.modal-error {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--destructive);
	}

	.modal-actions {
		display: flex;
		justify-content: flex-end;
		gap: 0.5rem;
		margin-top: 0.5rem;
	}

	.btn-cancel {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		background: none;
		border: 1px solid var(--border);
		padding: 0.4rem 0.75rem;
		border-radius: 0.5rem;
		cursor: pointer;
		transition: all 0.2s ease;
	}

	.btn-cancel:hover {
		color: var(--text-secondary);
		border-color: var(--border);
	}

	.btn-create {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--foreground);
		background: var(--card);
		border: 1px solid var(--border);
		padding: 0.4rem 0.75rem;
		border-radius: 0.5rem;
		cursor: pointer;
		transition: all 0.2s ease;
	}

	.btn-create:hover:not(:disabled) {
		background: var(--card);
	}

	.btn-create:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

/* Little Moon surfaces, controls, and readable content. */

.modal { background: var(--popover); border-color: var(--border); max-height: calc(100dvh - 40px); overflow-y: auto; }
.modal-title { font: 400 28px var(--font-display); }
.modal-close { width: 44px; height: 44px; }
.field-input, .field-textarea { font-size: 16px; min-height: 44px; background: var(--card); border-color: var(--input); min-width: 0; width: 100%; }
.field-name { min-width: 0; }
.field-label { font-size: 14px; }
.field-hint { display: block; margin: 4px 0 0; font-size: 12px; }
.btn-create { background: var(--primary); color: var(--primary-foreground); border-color: var(--primary); }
.btn-create:hover:not(:disabled) { background: var(--primary); color: var(--primary-foreground); filter: brightness(1.06); }
.btn-cancel { background: var(--card); border-color: var(--border); color: var(--foreground); }
.modal-error { color: var(--destructive); font-size: 14px; }

button { min-height: 44px; font-family: var(--font-body); }

button:focus-visible, input:focus-visible, textarea:focus-visible { outline: 2px solid var(--ring); outline-offset: 3px; }

@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; transition: none !important; } }

.modal { margin: auto; color: var(--foreground); }
.modal::backdrop { background: color-mix(in srgb, var(--background) 80%, transparent); }
</style>
