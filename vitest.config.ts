import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Component tests run in jsdom, so `svelte` has to resolve to its client build:
// without the browser condition `mount()` throws lifecycle_function_unavailable.
export default defineConfig({
  plugins: [svelte()],
  resolve: { conditions: ['browser'] },
  test: { environment: 'node', include: ['tests/**/*.test.ts', 'src/**/*.test.ts'] }
});
