import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  // Same-origin /ws in development too: forward to the daemon's human listener.
  server: { host: true, proxy: { '/ws': { target: 'ws://127.0.0.1:7778', ws: true } } },
})
