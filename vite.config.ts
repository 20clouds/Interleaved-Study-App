import { resolve } from 'path'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// The page is served to the Tauri webview: a fixed port in dev, a static folder in the built app.
export default defineConfig({
  root: resolve(__dirname, 'src/renderer'),
  plugins: [react()],
  clearScreen: false,
  server: { port: 5183, strictPort: true, host: '127.0.0.1' },
  build: { outDir: resolve(__dirname, 'dist'), emptyOutDir: true, target: 'safari15' }
})
