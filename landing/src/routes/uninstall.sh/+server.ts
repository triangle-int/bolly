import type { RequestHandler } from './$types.js';

const SCRIPT_URL = 'https://raw.githubusercontent.com/triangle-int/nolune/main/scripts/uninstall.sh';

export const GET: RequestHandler = async () => {
	const res = await fetch(SCRIPT_URL);
	if (!res.ok) {
		return new Response('Failed to fetch uninstall script', { status: 502 });
	}
	const script = await res.text();
	return new Response(script, {
		headers: {
			'Content-Type': 'text/plain; charset=utf-8',
			'Cache-Control': 'public, max-age=300',
		},
	});
};
