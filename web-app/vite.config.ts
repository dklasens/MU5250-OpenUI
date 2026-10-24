import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  base: './',
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          // jsQR is loaded on demand by the SIM Management add-eSIM panel only; keep it out of
          // the vendor chunk every page loads.
          if (id.includes('node_modules/jsqr/')) return 'jsqr'
          if (id.includes('node_modules')) return 'vendor'
        },
      },
    },
  },
})
