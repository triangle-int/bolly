import { track } from '@vercel/analytics/server';
import type { RequestHandler } from './$types.js';

const SCRIPT_URL = 'https://raw.githubusercontent.com/triangle-int/nolune/main/scripts/install.sh';

export const GET: RequestHandler = async ({ request }) => {
	const res = await fetch(SCRIPT_URL);
	if (!res.ok) {
		return new Response('Failed to fetch install script', { status: 502 });
	}

	track('install_script_download', {}, { request }).catch(() => {});

	const script = await res.text();
	return new Response(script, {
		headers: {
			'Content-Type': 'text/plain; charset=utf-8',
			'Cache-Control': 'public, max-age=300',
		},
	});
};
