<script lang="ts">
	import Footer from '$lib/components/Footer.svelte';
</script>

<svelte:head>
	<title>Privacy Policy — Nolune</title>
</svelte:head>

<div class="legal-page">
	<header class="legal-header">
		<div class="legal-header-inner">
			<a href="/" class="legal-brand">
				<div class="legal-logo">b</div>
				<span class="legal-brand-name">nolune</span>
			</a>
		</div>
	</header>

	<div class="legal-content">
		<h1 class="legal-title">Privacy Policy</h1>
		<p class="legal-date">Last updated: March 13, 2026</p>

		<div class="prose">
			<h2>1. Who we are</h2>
			<p>
				Nolune is operated by Triangle Interactive ("we", "us", "our"). We provide an AI companion
				platform that runs on dedicated server instances. This policy explains how we collect, use,
				and protect your information.
			</p>

			<h2>2. Information we collect</h2>

			<h3>Account information</h3>
			<p>When you create an account, we collect your email address and a hashed password. If you subscribe to a paid plan, Stripe processes your payment — we do not store credit card numbers.</p>

			<h3>Companion data</h3>
			<p>
				Your companion instance runs on a dedicated server with its own persistent storage. Conversations, memories, files, and any content you share with your companion are stored on that instance. We do not access, read, or analyze your companion data except when required to provide technical support you explicitly request, or when required by law.
			</p>

			<h3>Google account data</h3>
			<p>
				If you connect your Google account, we store an OAuth refresh token and access token to enable Gmail, Google Calendar, and Google Drive integrations. These tokens are used solely to perform actions you or your companion initiate (sending emails, reading your calendar, accessing drive files). We do not read, store, or analyze the content of your emails, calendar events, or drive files on our servers — your companion processes this data in real-time on your dedicated instance.
			</p>

			<h3>Usage data</h3>
			<p>We collect basic usage metrics: message counts and token usage per instance for rate limiting and billing. We use server logs for debugging and security. We do not use analytics trackers or third-party tracking scripts.</p>

			<h2>3. How we use your information</h2>
			<ul>
				<li>To provide and maintain your companion instance</li>
				<li>To process payments via Stripe</li>
				<li>To enforce rate limits and prevent abuse</li>
				<li>To send transactional emails (password resets, billing notifications)</li>
				<li>To facilitate Google integrations you explicitly authorize</li>
			</ul>
			<p>We do not sell your data. We do not use your data to train AI models. We do not share your data with third parties except as described below.</p>

			<h2>4. Third-party services</h2>
			<ul>
				<li><strong>Fly.io</strong> — hosts your companion instances</li>
				<li><strong>Neon</strong> — hosts our account database</li>
				<li><strong>Stripe</strong> — processes payments</li>
				<li><strong>Cloudflare</strong> — DNS and TLS for your subdomain</li>
				<li><strong>OpenRouter / OpenAI / Anthropic</strong> — LLM providers that process your companion's conversations (subject to their respective privacy policies)</li>
				<li><strong>Google APIs</strong> — Gmail, Calendar, and Drive access when you connect your Google account (subject to <a href="https://policies.google.com/privacy" target="_blank">Google's Privacy Policy</a>)</li>
			</ul>

			<h2>5. Google API Services User Data Policy</h2>
			<p>
				Nolune's use and transfer of information received from Google APIs adheres to the
				<a href="https://developers.google.com/terms/api-services-user-data-policy" target="_blank">Google API Services User Data Policy</a>,
				including the Limited Use requirements. Specifically:
			</p>
			<ul>
				<li>We only request access to the Google API scopes necessary for the features you use</li>
				<li>We do not use Google user data for advertising</li>
				<li>We do not transfer Google user data to third parties except as necessary to provide the service or as required by law</li>
				<li>We do not use Google user data to train AI models</li>
				<li>Your Google data is processed on your dedicated companion instance and is not stored on shared infrastructure</li>
			</ul>

			<h2>6. Data retention</h2>
			<p>
				Your account data is retained as long as your account is active. Your companion data is stored on your dedicated instance volume. If you cancel your subscription, your instance is stopped and data is retained for 30 days before deletion. You can request immediate deletion by contacting us.
			</p>
			<p>Google OAuth tokens are deleted immediately when you disconnect your Google account from the dashboard.</p>

			<h2>7. Data security</h2>
			<p>
				All connections are encrypted via TLS. Passwords are hashed. API keys and tokens are stored encrypted at rest. Each companion keeps its data in a separate local workspace. We follow industry-standard security practices, but no system is perfectly secure.
			</p>

			<h2>8. Your rights</h2>
			<p>You can:</p>
			<ul>
				<li>Access, export, or delete your companion data at any time via the companion's tools</li>
				<li>Disconnect your Google account at any time from the dashboard</li>
				<li>Delete your account by contacting us</li>
				<li>Request a copy of your personal data</li>
			</ul>

			<h2>9. Changes</h2>
			<p>We may update this policy. Material changes will be communicated via email or a notice on the dashboard.</p>

			<h2>10. Contact</h2>
			<p>Questions? Email <a href="mailto:support@nolune.dev">support@nolune.dev</a>.</p>
		</div>
	</div>

	<Footer />
</div>

<style>
	.legal-page { min-height: 100dvh; background: oklch(0.04 0.015 260); }
	.legal-header { border-bottom: 1px solid oklch(1 0 0 / 6%); background: oklch(0.05 0.015 260 / 70%); backdrop-filter: blur(24px); }
	.legal-header-inner { max-width: 900px; margin: 0 auto; padding: 1rem 1.5rem; }
	.legal-brand { display: flex; align-items: center; gap: 0.625rem; text-decoration: none; width: fit-content; }
	.legal-logo { width: 1.75rem; height: 1.75rem; border-radius: 0.375rem; display: flex; align-items: center; justify-content: center; font-family: var(--font-display); font-style: italic; font-size: 0.875rem; color: var(--color-warm); background: oklch(1 0 0 / 5%); border: 1px solid oklch(1 0 0 / 10%); }
	.legal-brand-name { font-family: var(--font-display); font-style: italic; font-size: 1.125rem; color: var(--color-text); }
	.legal-content { max-width: 700px; margin: 0 auto; padding: 4rem 1.5rem; }
	.legal-title { font-family: var(--font-display); font-style: italic; font-size: 1.875rem; font-weight: 400; color: var(--color-text); margin-bottom: 0.5rem; }
	.legal-date { font-size: 0.75rem; color: oklch(0.90 0.02 75 / 25%); margin-bottom: 3rem; }

	.prose {
		color: oklch(0.65 0.02 280);
		line-height: 1.75;
	}
	.prose h2 {
		color: oklch(0.88 0.02 280);
		font-size: 1.15rem;
		font-weight: 600;
		margin-top: 2.5rem;
		margin-bottom: 0.75rem;
	}
	.prose h3 {
		color: oklch(0.78 0.02 280);
		font-size: 0.95rem;
		font-weight: 600;
		margin-top: 1.5rem;
		margin-bottom: 0.5rem;
	}
	.prose p {
		font-size: 0.875rem;
		margin-bottom: 1rem;
	}
	.prose ul {
		font-size: 0.875rem;
		padding-left: 1.5rem;
		margin-bottom: 1rem;
	}
	.prose li {
		margin-bottom: 0.35rem;
	}
	.prose a {
		color: oklch(0.78 0.12 75);
		text-decoration: underline;
		text-underline-offset: 2px;
	}
	.prose a:hover {
		color: oklch(0.85 0.12 75);
	}
</style>
