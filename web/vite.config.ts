import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [svelte()],
  server: {
    // `pnpm dev` のときは、API を別に起動した lapper-server に中継する
    proxy: {
      '/api': 'http://127.0.0.1:3000',
    },
  },
})
