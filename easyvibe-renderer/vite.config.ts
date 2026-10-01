import path from "path"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"
import { inspectAttr } from 'kimi-plugin-inspect-react'

// https://vite.dev/config/
export default defineConfig(({ command }) => ({
  base: './',
  // M4-1.5：inspectAttr（code-path DOM 标注）仅开发模式——生产构建不带调试脚手架
  plugins: [command === 'serve' ? inspectAttr() : null, react()].filter(Boolean),
  server: {
    port: 3000,
    proxy: {
      '/api': { target: 'http://127.0.0.1:7101', changeOrigin: true },
      '/ws': { target: 'ws://127.0.0.1:7101', ws: true },
    },
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
}));
