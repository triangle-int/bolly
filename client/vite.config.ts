import tailwindcss from "@tailwindcss/vite";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

export default defineConfig({
	plugins: [
		tailwindcss(),
		sveltekit(),
	],
	build: {
		target: "esnext",
	},
	server: {
		proxy: {
			"/api/ws": { target: "ws://localhost:26559", ws: true },
			"/api": "http://localhost:26559",
		},
	},
});
