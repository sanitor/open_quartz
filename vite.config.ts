import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { resolve } from 'node:path'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  resolve: {
    alias: {
      '@nodes/yolo-detector': resolve(__dirname, 'rust/crates/yolo-detector/pkg/yolo_detector.js'),
    },
  },
  assetsInclude: ['**/*.wasm'],
})
