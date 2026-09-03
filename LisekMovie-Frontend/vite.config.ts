import { defineConfig } from 'vitest/config';
import tailwindcss from '@tailwindcss/vite';
import adapter from '@sveltejs/adapter-node';
import { sveltekit } from '@sveltejs/kit/vite';

export default defineConfig({
	plugins: [
		tailwindcss(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			adapter: adapter()
		})
	],
	server: {
		// Forward /api/* to the Rust backend during `npm run dev`, both for
		// browser fetches and for SvelteKit's server-side `load` fetches
		// (those resolve relative URLs against this same dev server, so the
		// proxy applies to them too). Production deployment reverse-proxies
		// the two behind one origin instead — see the deployment notes
		// planned for the packaging phase.
		proxy: {
			'/api': 'http://localhost:8080'
		}
	},
	test: {
		expect: { requireAssertions: true },
		projects: [
			{
				extends: './vite.config.ts',
				test: {
					name: 'server',
					environment: 'node',
					include: ['src/**/*.{test,spec}.{js,ts}'],
					exclude: ['src/**/*.svelte.{test,spec}.{js,ts}']
				}
			},
			// Runda 10: the first Svelte-component-level tests in this
			// project (`*.svelte.test.ts`) — proving the `/library/[id]`
			// and `/media/[id]` navigation fix actually re-fetches and
			// clears stale content on an id change requires mounting the
			// real component and simulating a route param change, which
			// the "server"/node project above can't do at all (it has no
			// DOM). See `vitest-setup-client.ts` and
			// `src/lib/testing/mockPageState.svelte.ts`.
			{
				extends: './vite.config.ts',
				// Without this, Vite resolves `svelte`'s package.json `exports`
				// using its default (node/SSR) conditions even under the jsdom
				// environment below — `mount()` (client-side) doesn't exist on
				// that build, only the server-side `render()` does, so every
				// `@testing-library/svelte` `render()` call fails with
				// "`mount(...)` is not available on the server". The `browser`
				// condition is what makes it resolve Svelte's actual client
				// runtime instead.
				resolve: {
					conditions: ['browser']
				},
				test: {
					name: 'client',
					environment: 'jsdom',
					include: ['src/**/*.svelte.{test,spec}.{js,ts}'],
					setupFiles: ['./vitest-setup-client.ts']
				}
			}
		]
	}
});
