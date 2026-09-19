<script lang="ts">
	import { cn } from "$lib/utils.js";
	import { watch } from "runed";
	import { onDestroy } from "svelte";
	import { AttachmentsContext, setAttachmentsContext } from "../context/attachments.svelte.js";
	import { getPromptInputProvider } from "../context/provider.svelte.js";
	import {
		setPromptInputTextRegistration,
		type PromptInputTextHandle,
	} from "../context/text-registration.svelte.js";
	import type { Message, PromptInputAttachment } from "../context/types.js";

	interface Props {
		class?: string;
		attachments?: PromptInputAttachment[];
		accept?: string;
		multiple?: boolean;


		clearOnSubmit?: boolean;
		resetFormOnSubmit?: boolean;
		maxFiles?: number;
		maxFileSize?: number; // bytes
		onError?: (err: {
			code: "max_files" | "max_file_size" | "accept";
			message: string;
		}) => void;
		onFileAdd?: (added: PromptInputAttachment[], attachments: PromptInputAttachment[]) => void;
		onFileRemove?: (
			removed: PromptInputAttachment[],
			attachments: PromptInputAttachment[]
		) => void;
		disabled?: boolean;
		onSubmit: (message: Message, event: SubmitEvent) => void | boolean | Promise<void | boolean>;
		children?: import("svelte").Snippet;
	}
	// indexing

	let {
		class: className,
		attachments = $bindable<PromptInputAttachment[] | undefined>(undefined),
		accept,
		multiple,
		clearOnSubmit = true,
		resetFormOnSubmit = false,
		maxFiles,
		maxFileSize,
		onError,
		onFileAdd,
		onFileRemove,
		onSubmit,
		disabled = false,
		children,
		...props
	}: Props = $props();

	let submitting = $state(false);
	let formRef = $state<HTMLFormElement | null>(null);
	let controller = getPromptInputProvider();
	let usingProvider = Boolean(controller);
	let localAttachmentsContext = new AttachmentsContext();
	let attachmentsContext = controller?.attachments ?? localAttachmentsContext;
	let promptTextHandle = $state<PromptInputTextHandle | null>(null);

	setPromptInputTextRegistration({
		register: (handle) => {
			promptTextHandle = handle;
		},
		unregister: (handle) => {
			if (promptTextHandle === handle) {
				promptTextHandle = null;
			}
		},
	});

	$effect(() => {
		attachmentsContext.configure({
			accept,
			multiple,
			maxFiles,
			maxFileSize,
			onError,
			onFileAdd,
			onFileRemove,
		});
	});

	$effect(() => {
		let syncAttachments = (next: PromptInputAttachment[]) => {
			if (attachments !== next) {
				attachments = next;
			}
		};

		attachmentsContext.onAttachmentsChange = syncAttachments;
		syncAttachments(attachmentsContext.attachments);

		return () => {
			if (attachmentsContext.onAttachmentsChange === syncAttachments) {
				attachmentsContext.onAttachmentsChange = undefined;
			}
		};
	});

	$effect(() => {
		if (attachments !== undefined && attachmentsContext.attachments !== attachments) {
			attachmentsContext.replace(attachments);
		}
	});

	// Attach drop handlers on nearest form
	watch(
		() => formRef,
		(formRef) => {
			if (!formRef) return;

			let onDragOver = (e: DragEvent) => {
				if (e.dataTransfer?.types?.includes("Files")) {
					e.preventDefault();
				}
			};

			let onDrop = (e: DragEvent) => {
				if (e.dataTransfer?.types?.includes("Files")) {
					e.preventDefault();
				}
				if (e.dataTransfer?.files && e.dataTransfer.files.length > 0) {
					if (!disabled && !submitting) attachmentsContext.add(e.dataTransfer.files);
				}
			};

			formRef.addEventListener("dragover", onDragOver);
			formRef.addEventListener("drop", onDrop);

			return () => {
				formRef?.removeEventListener("dragover", onDragOver);
				formRef?.removeEventListener("drop", onDrop);
			};
		}
	);

	let handleChange = (event: Event) => {
		let target = event.currentTarget as HTMLInputElement;
		if (target.files && !disabled && !submitting) {
			attachmentsContext.add(target.files);
		}
		target.value = "";
	};

	let handleSubmit = async (event: SubmitEvent) => {
		event.preventDefault();
		if (disabled || submitting) return;

		let form = event.currentTarget as HTMLFormElement;
		let text = usingProvider
			? (controller?.textInput.value ?? "")
			: (promptTextHandle?.getValue() ??
				((new FormData(form).get("message") as string) || ""));
		let submittedAttachments = attachmentsContext.attachments.map((attachment) => ({
			...attachment,
		}));
		if (!text.trim() && submittedAttachments.length === 0) return;
		submitting = true;

		try {
			let result = await onSubmit(
				{
					text,
					attachments: submittedAttachments,
				},
				event
			);

			if (result === false) return;

			// Only clear if submission was successful
			if (clearOnSubmit) {
				attachmentsContext.clear();
				if (usingProvider) {
					controller?.textInput.clear();
				} else {
					promptTextHandle?.clear();
				}

				if (resetFormOnSubmit) {
					form.reset();
				}
			}
		} catch (error) {
			// Don't clear on error - user may want to retry
			console.error("Submit failed:", error);
		} finally {
			submitting = false;
		}
	};

	onDestroy(() => {
		if (usingProvider) {
			attachmentsContext.onAttachmentsChange = undefined;
			attachmentsContext.onFileAdd = undefined;
			attachmentsContext.onFileRemove = undefined;
			attachmentsContext.onError = undefined;
			if (attachmentsContext.fileInputRef) {
				attachmentsContext.fileInputRef = null;
			}
			return;
		}

		localAttachmentsContext.destroy();
	});

	setAttachmentsContext(attachmentsContext);
</script>

<input
	{accept}
	disabled={disabled || submitting}
	class="hidden"
	{multiple}
	onchange={handleChange}
	bind:this={attachmentsContext.fileInputRef}
	type="file"
/>
<form
	bind:this={formRef}
	aria-busy={submitting}
	class={cn("bg-background w-full overflow-hidden rounded-xl border shadow-sm", className)}
	onsubmit={handleSubmit}
	{...props}
>
	{#if children}
		{@render children()}
	{/if}
</form>
