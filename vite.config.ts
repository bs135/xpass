import {defineConfig} from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
    plugins: [react(), tailwindcss()],
    // Keep Tauri CLI output visible in the terminal during `tauri dev`.
    clearScreen: false,
    server: {
        // tauri.conf.json devUrl points at this port, so it must not drift.
        port: 5173,
        strictPort: true,
        watch: {ignored: ['**/src-tauri/**', '**/crates/**', '**/spikes/**', '**/target/**']},
    },
    build: {
        target: 'es2022',
        // No source maps in the shipped bundle.
        sourcemap: false,
    },
    test: {
        environment: 'node',
        include: ['src/**/*.test.ts', 'scripts/**/*.test.mjs'],
    },
})
