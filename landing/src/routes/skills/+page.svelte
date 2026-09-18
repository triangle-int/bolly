<script lang="ts">
	import Nav from '$lib/components/Nav.svelte';
	import Footer from '$lib/components/Footer.svelte';
	import Reveal from '$lib/components/Reveal.svelte';

	interface RegistrySkill {
		id: string;
		name: string;
		description: string;
		icon: string;
		repo: string;
		git_ref: string;
		author: string;
		path?: string;
	}

	const REGISTRY_URL =
		'https://raw.githubusercontent.com/triangle-int/bolly-skills/main/registry.json';

	let skills = $state<RegistrySkill[]>([]);
	let loading = $state(true);
	let error = $state('');
	let search = $state('');
	let selectedSkill = $state<RegistrySkill | null>(null);

	$effect(() => {
		fetchSkills();
	});

	async function fetchSkills() {
		loading = true;
		error = '';
		try {
			const resp = await fetch(REGISTRY_URL);
			if (!resp.ok) throw new Error(`Failed to load (${resp.status})`);
			skills = await resp.json();
		} catch (e) {
			error = e instanceof Error ? e.message : 'Failed to load skills';
		} finally {
			loading = false;
		}
	}

	let filtered = $derived(
		search.trim()
			? skills.filter(
					(s) =>
						s.name.toLowerCase().includes(search.toLowerCase()) ||
						s.description.toLowerCase().includes(search.toLowerCase()) ||
						s.author.toLowerCase().includes(search.toLowerCase())
				)
			: skills
	);
</script>

<svelte:head>
	<title>Skills Library — Nolune</title>
	<meta
		name="description"
		content="Browse community skills for Nolune. Teach your companion new abilities — from code review to creative writing."
	/>
</svelte:head>

<Nav />

<main class="pt-28 pb-20 min-h-screen">
	<div class="mx-auto max-w-[1100px] px-6">
		<!-- Header -->
		<Reveal>
			<p class="section-label">Community</p>
		</Reveal>
		<Reveal delay={100}>
			<h1 class="section-title">skills library</h1>
		</Reveal>
		<Reveal delay={200}>
			<p class="section-desc">
				Skills teach your companion new abilities. Browse what the community has
				built, or
				<a href="https://github.com/triangle-int/bolly-skills" target="_blank" class="text-warm hover:underline">
					publish your own
				</a>.
			</p>
		</Reveal>

		<!-- Search -->
		<Reveal delay={300}>
			<div class="search-wrap">
				<svg
					class="search-icon"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="1.5"
					stroke-linecap="round"
					stroke-linejoin="round"
				>
					<circle cx="11" cy="11" r="8" />
					<path d="m21 21-4.35-4.35" />
				</svg>
				<input
					type="text"
					class="search-input"
					placeholder="Search skills..."
					bind:value={search}
				/>
				{#if search}
					<button class="search-clear" onclick={() => (search = '')} aria-label="Clear search">
						<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" class="w-3.5 h-3.5">
							<path d="M18 6 6 18M6 6l12 12" />
						</svg>
					</button>
				{/if}
			</div>
		</Reveal>

		<!-- Content -->
		{#if loading}
			<div class="loading-state">
				<div class="loading-dot"></div>
				<p class="loading-text">loading skills...</p>
			</div>
		{:else if error}
			<div class="empty-state">
				<p class="empty-title">couldn't load skills</p>
				<p class="empty-hint">{error}</p>
				<button class="btn-retry" onclick={fetchSkills}>try again</button>
			</div>
		{:else if filtered.length === 0}
			<div class="empty-state">
				{#if search}
					<p class="empty-title">no skills match "{search}"</p>
					<p class="empty-hint">try a different search term</p>
				{:else}
					<p class="empty-title">no skills published yet</p>
					<p class="empty-hint">be the first to contribute</p>
				{/if}
			</div>
		{:else}
			<Reveal delay={400}>
				<p class="results-count">
					{filtered.length} skill{filtered.length !== 1 ? 's' : ''}
				</p>
			</Reveal>

			<Reveal delay={450}>
				<div class="skills-grid">
					{#each filtered as skill (skill.id)}
						<button
							class="skill-card"
							class:skill-card-selected={selectedSkill?.id === skill.id}
							onclick={() =>
								(selectedSkill =
									selectedSkill?.id === skill.id ? null : skill)}
						>
							<div class="skill-header">
								<span class="skill-icon">{skill.icon || '~'}</span>
								<h3 class="skill-name">{skill.name}</h3>
							</div>
							<p class="skill-desc">{skill.description}</p>
							<div class="skill-meta">
								{#if skill.author}
									<span class="skill-author">{skill.author}</span>
								{/if}
								<span class="skill-repo">{skill.repo}</span>
							</div>

							{#if selectedSkill?.id === skill.id}
								<div class="skill-details">
									<p class="skill-install-hint">
										Install from the <strong>Skills</strong> tab in your Nolune instance, or browse the source:
									</p>
									<a
										href="https://github.com/{skill.repo}{skill.path ? `/tree/${skill.git_ref}/${skill.path}` : ''}"
										target="_blank"
										class="skill-github-link"
										onclick={(e) => e.stopPropagation()}
									>
										view on GitHub &rarr;
									</a>
								</div>
							{/if}
						</button>
					{/each}
				</div>
			</Reveal>
		{/if}

		<!-- Publish CTA -->
		<Reveal delay={600}>
			<section class="publish-cta">
				<h2 class="publish-title">publish a skill</h2>
				<p class="publish-desc">
					Create a GitHub repo with a <code>skill.json</code> manifest and
					<code>SKILL.md</code>, then open a PR to the registry.
				</p>
				<a
					href="https://github.com/triangle-int/bolly-skills"
					target="_blank"
					class="btn-primary"
				>
					Contribute
					<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" class="w-4 h-4">
						<path d="M5 12h14M12 5l7 7-7 7" />
					</svg>
				</a>
			</section>
		</Reveal>
	</div>
</main>

<Footer />

<style>
	.section-label {
		font-size: 0.8rem;
		letter-spacing: 0.15em;
		text-transform: uppercase;
		color: var(--color-warm-dim);
		margin-bottom: 1rem;
	}

	.section-title {
		font-family: var(--font-display);
		font-weight: 400;
		font-style: italic;
		font-size: clamp(1.75rem, 3.5vw, 2.5rem);
		line-height: 1.15;
		letter-spacing: -0.02em;
		color: var(--color-text);
		margin-bottom: 1rem;
	}

	.section-desc {
		font-size: 0.9375rem;
		color: var(--color-text-dim);
		max-width: 480px;
		line-height: 1.6;
		margin-bottom: 2rem;
	}

	/* Search — glass */
	.search-wrap {
		position: relative;
		max-width: 400px;
		margin-bottom: 2rem;
	}

	.search-icon {
		position: absolute;
		left: 0.875rem;
		top: 50%;
		transform: translateY(-50%);
		width: 1rem;
		height: 1rem;
		color: oklch(0.90 0.02 75 / 25%);
		pointer-events: none;
	}

	.search-input {
		width: 100%;
		font-family: var(--font-mono);
		font-size: 0.9rem;
		color: var(--color-text);
		background: var(--glass-bg);
		backdrop-filter: var(--glass-blur);
		border: 1px solid var(--glass-border);
		border-top-color: var(--glass-border-top);
		border-radius: 0.75rem;
		padding: 0.625rem 2.5rem 0.625rem 2.5rem;
		outline: none;
		transition: all 0.3s ease;
	}

	.search-input:focus {
		border-color: oklch(1 0 0 / 16%);
		box-shadow: 0 0 0 3px oklch(0.55 0.08 240 / 6%);
	}

	.search-input::placeholder {
		color: oklch(0.90 0.02 75 / 20%);
	}

	.search-clear {
		position: absolute;
		right: 0.625rem;
		top: 50%;
		transform: translateY(-50%);
		color: oklch(0.90 0.02 75 / 25%);
		background: none;
		border: none;
		cursor: pointer;
		padding: 0.25rem;
		display: flex;
		transition: color 0.2s;
	}

	.search-clear:hover {
		color: oklch(0.90 0.02 75 / 60%);
	}

	/* Loading / Empty */
	.loading-state {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 1rem;
		padding: 6rem 0;
	}

	.loading-dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--color-warm-dim);
		animation: presence-beacon 2.5s ease-in-out infinite;
	}

	.loading-text {
		font-family: var(--font-mono);
		font-size: 0.85rem;
		color: oklch(0.90 0.02 75 / 20%);
	}

	.empty-state {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 0.75rem;
		padding: 6rem 0;
		text-align: center;
	}

	.empty-title {
		font-family: var(--font-display);
		font-style: italic;
		font-size: 1.125rem;
		color: var(--color-text-dim);
	}

	.empty-hint {
		font-size: 0.9rem;
		color: oklch(0.90 0.02 75 / 25%);
	}

	.btn-retry {
		font-family: var(--font-mono);
		font-size: 0.85rem;
		color: oklch(0.90 0.02 75 / 40%);
		background: var(--glass-bg);
		backdrop-filter: var(--glass-blur);
		border: 1px solid var(--glass-border);
		padding: 0.4rem 1rem;
		border-radius: 0.5rem;
		cursor: pointer;
		transition: all 0.2s ease;
		margin-top: 0.5rem;
	}

	.btn-retry:hover {
		color: var(--color-warm);
		border-color: oklch(1 0 0 / 14%);
	}

	/* Results */
	.results-count {
		font-family: var(--font-mono);
		font-size: 0.8rem;
		color: oklch(0.90 0.02 75 / 20%);
		letter-spacing: 0.04em;
		margin-bottom: 1rem;
	}

	/* Grid — glass cards */
	.skills-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
		gap: 1rem;
		margin-bottom: 4rem;
	}

	/* Card — glass */
	.skill-card {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		padding: 1.5rem 1.375rem;
		background: var(--glass-bg);
		backdrop-filter: var(--glass-blur);
		border: 1px solid var(--glass-border);
		border-top-color: var(--glass-border-top);
		border-radius: 1rem;
		cursor: pointer;
		text-align: left;
		width: 100%;
		transition: all 0.4s cubic-bezier(0.16, 1, 0.3, 1);
		position: relative;
		overflow: hidden;
	}

	/* specular highlight */
	.skill-card::before {
		content: '';
		position: absolute;
		top: 0;
		left: 10%;
		right: 10%;
		height: 1px;
		background: linear-gradient(90deg, transparent, var(--glass-highlight), transparent);
		pointer-events: none;
		z-index: 2;
	}

	/* inner refraction */
	.skill-card::after {
		content: '';
		position: absolute;
		top: 0;
		left: 0;
		right: 0;
		height: 40%;
		background: linear-gradient(180deg, oklch(1 0 0 / 3%) 0%, transparent 100%);
		pointer-events: none;
		z-index: 1;
		border-radius: 1rem 1rem 0 0;
	}

	.skill-card:hover {
		border-color: oklch(1 0 0 / 14%);
		background: oklch(1 0 0 / 6%);
		transform: translateY(-2px);
		box-shadow:
			0 4px 24px oklch(0 0 0 / 20%),
			inset 0 1px 0 oklch(1 0 0 / 8%);
	}

	.skill-card-selected {
		border-color: oklch(1 0 0 / 12%);
		background: oklch(1 0 0 / 5%);
	}

	.skill-header {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		position: relative;
		z-index: 3;
	}

	.skill-icon {
		font-family: var(--font-mono);
		font-size: 0.9rem;
		color: oklch(0.90 0.02 75 / 40%);
		width: 2rem;
		height: 2rem;
		border-radius: 0.5rem;
		background: var(--glass-bg);
		backdrop-filter: var(--glass-blur);
		border: 1px solid var(--glass-border);
		border-top-color: var(--glass-border-top);
		display: flex;
		align-items: center;
		justify-content: center;
		flex-shrink: 0;
		transition: all 0.4s ease;
	}

	.skill-card:hover .skill-icon {
		background: oklch(1 0 0 / 8%);
		border-color: oklch(1 0 0 / 18%);
		color: oklch(0.90 0.02 75 / 75%);
		box-shadow: 0 0 20px oklch(0.55 0.08 240 / 8%);
	}

	.skill-name {
		font-family: var(--font-display);
		font-style: italic;
		font-weight: 400;
		font-size: 1rem;
		color: var(--color-text);
		letter-spacing: -0.01em;
		position: relative;
		z-index: 3;
	}

	.skill-desc {
		font-size: 0.9rem;
		color: var(--color-text-dim);
		line-height: 1.55;
		position: relative;
		z-index: 3;
	}

	.skill-meta {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		margin-top: 0.25rem;
		position: relative;
		z-index: 3;
	}

	.skill-author {
		font-family: var(--font-mono);
		font-size: 0.8rem;
		color: oklch(0.90 0.02 75 / 25%);
	}

	.skill-repo {
		font-family: var(--font-mono);
		font-size: 0.8rem;
		color: oklch(0.78 0.12 75 / 30%);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	/* Expanded details */
	.skill-details {
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
		padding-top: 0.75rem;
		margin-top: 0.25rem;
		border-top: 1px solid var(--glass-border);
		animation: fade-up 0.3s cubic-bezier(0.16, 1, 0.3, 1);
		position: relative;
		z-index: 3;
	}

	.skill-install-hint {
		font-size: 0.875rem;
		color: var(--color-text-dim);
		line-height: 1.5;
	}

	.skill-install-hint strong {
		color: oklch(0.78 0.12 75 / 50%);
	}

	.skill-github-link {
		font-family: var(--font-mono);
		font-size: 0.85rem;
		color: oklch(0.90 0.02 75 / 30%);
		text-decoration: none;
		transition: color 0.2s;
	}

	.skill-github-link:hover {
		color: var(--color-warm);
	}

	/* Publish CTA — glass */
	.publish-cta {
		text-align: center;
		padding: 4rem 2rem;
		background: var(--glass-bg);
		backdrop-filter: var(--glass-blur);
		border: 1px solid var(--glass-border);
		border-top-color: var(--glass-border-top);
		border-radius: 1rem;
		position: relative;
		overflow: hidden;
	}

	.publish-cta::before {
		content: '';
		position: absolute;
		top: 0;
		left: 10%;
		right: 10%;
		height: 1px;
		background: linear-gradient(90deg, transparent, var(--glass-highlight), transparent);
		pointer-events: none;
	}

	.publish-cta::after {
		content: '';
		position: absolute;
		top: 0;
		left: 0;
		right: 0;
		height: 40%;
		background: linear-gradient(180deg, oklch(1 0 0 / 3%) 0%, transparent 100%);
		pointer-events: none;
		border-radius: 1rem 1rem 0 0;
	}

	.publish-title {
		font-family: var(--font-display);
		font-style: italic;
		font-weight: 400;
		font-size: 1.5rem;
		color: var(--color-text);
		margin-bottom: 0.75rem;
		position: relative;
		z-index: 2;
	}

	.publish-desc {
		font-size: 0.875rem;
		color: var(--color-text-dim);
		max-width: 420px;
		margin: 0 auto 1.5rem;
		line-height: 1.6;
		position: relative;
		z-index: 2;
	}

	.publish-desc code {
		font-family: var(--font-mono);
		font-size: 0.9rem;
		color: oklch(0.78 0.12 75 / 45%);
		background: oklch(1 0 0 / 4%);
		padding: 0.1rem 0.35rem;
		border-radius: 0.25rem;
		border: 1px solid var(--glass-border);
	}

	@media (max-width: 768px) {
		.skills-grid {
			grid-template-columns: 1fr;
		}

		.search-wrap {
			max-width: 100%;
		}
	}
</style>
