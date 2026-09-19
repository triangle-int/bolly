<script lang="ts">
	import Footer from '$lib/components/Footer.svelte';
</script>

<svelte:head>
	<title>Terms of Service — Nolune</title>
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
		<h1 class="legal-title">Terms of Service</h1>
		<p class="legal-date">Last updated: March 13, 2026</p>

		<div class="prose">
			<h2>1. Agreement</h2>
			<p>
				By using Nolune ("the Service"), operated by Triangle Interactive ("we", "us"), you agree to these terms.
				If you don't agree, don't use the Service.
			</p>

			<h2>2. The Service</h2>
			<p>
				Nolune provides AI companion instances that run on dedicated servers. Each companion has persistent memory,
				tool access and optional installable skills. The companion uses third-party LLM
				providers to generate responses.
			</p>

			<h2>3. Accounts</h2>
			<p>You must provide a valid email address to create an account. You are responsible for keeping your account credentials secure. One person per account. You must be at least 13 years old to use the Service.</p>

			<h2>4. Subscriptions and billing</h2>
			<p>
				Paid plans are billed monthly via Stripe. You can cancel at any time — your instance remains active until the end of the billing period.
				Refunds are handled on a case-by-case basis. We reserve the right to change pricing with 30 days notice.
			</p>

			<h2>5. Your data</h2>
			<p>
				You own your data. Conversations, files, memories, and anything you create with your companion belong to you.
				We do not access your companion data except for technical support you explicitly request, or when required by law.
				See our <a href="/privacy">Privacy Policy</a> for details.
			</p>

			<h2>6. Optional Google Workspace skill</h2>
			<p>
				Nolune does not include a Google account connection or token broker. If you install the optional official
				<code>gog</code> skill, you configure and authorize the external gog CLI yourself. gog owns those credentials
				locally through its configured keyring or credential backend; Nolune does not copy or store them. Review every side effect and revoke access with
				gog or from your <a href="https://myaccount.google.com/permissions" target="_blank">Google account settings</a>.
			</p>

			<h2>7. Acceptable use</h2>
			<p>You agree not to:</p>
			<ul>
				<li>Use the Service for illegal activity</li>
				<li>Send spam or bulk unsolicited messages via the email tools</li>
				<li>Attempt to access other users' instances or data</li>
				<li>Abuse rate limits or circumvent billing</li>
				<li>Use the Service to generate content that violates the LLM providers' usage policies</li>
				<li>Resell access to the Service without permission</li>
			</ul>
			<p>We may suspend or terminate accounts that violate these terms.</p>

			<h2>8. AI limitations</h2>
			<p>
				Your companion is an AI. It can make mistakes, hallucinate, or produce inaccurate information.
				It should not be relied upon for medical, legal, financial, or safety-critical decisions.
				Actions taken by your companion (sending emails, creating events, running commands) are your responsibility.
			</p>

			<h2>9. Availability</h2>
			<p>
				We aim for high availability but do not guarantee uninterrupted service. Maintenance, infrastructure
				issues, or third-party outages (Fly.io, LLM providers, Google APIs) may cause downtime. We are not
				liable for losses resulting from service interruptions.
			</p>

			<h2>10. Limitation of liability</h2>
			<p>
				The Service is provided "as is" without warranties of any kind. To the maximum extent permitted by law,
				we are not liable for any indirect, incidental, special, or consequential damages arising from your use
				of the Service. Our total liability is limited to the amount you paid us in the 12 months preceding the claim.
			</p>

			<h2>11. Termination</h2>
			<p>
				You can delete your account at any time. We can terminate your account for violation of these terms
				with notice. Upon termination, your companion instances are stopped and data is deleted after 30 days.
			</p>

			<h2>12. Changes</h2>
			<p>We may update these terms. Material changes will be communicated via email. Continued use after changes take effect constitutes acceptance.</p>

			<h2>13. Contact</h2>
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
