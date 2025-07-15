import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
  ],
  server: {
    proxy: {
      '/version': {
        target: 'http://localhost:5800',
        changeOrigin: true,
      },
      '/sub': {
        target: 'http://localhost:5800',
        ws: true,
      }
    }
  }
})
