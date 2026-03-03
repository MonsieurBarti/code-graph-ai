import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [svelte(), tailwindcss()],
  server: {
    proxy: {
      '/api': 'http://127.0.0.1:7070',
      '/ws': { target: 'ws://127.0.0.1:7070', ws: true },
    },
  },
});
