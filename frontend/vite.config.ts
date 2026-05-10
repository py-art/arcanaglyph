import { defineConfig } from 'vite';
import { resolve } from 'node:path';

// Vite config для Tauri webview.
// Tauri запускает `npm run dev` в dev-mode и грузит UI с http://127.0.0.1:5173
// (см. tauri.conf.json `devUrl`); в release-сборке использует
// `frontend/dist/` (см. `frontendDist`).
//
// Многостраничная сборка: index.html — главное окно приложения,
// widget.html — отдельное окно GNOME-виджета записи.
export default defineConfig({
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'esnext',
    rollupOptions: {
      input: {
        main: resolve(__dirname, 'index.html'),
        widget: resolve(__dirname, 'widget.html'),
      },
    },
  },
  server: {
    port: 5173,
    strictPort: true,
    host: '127.0.0.1',
  },
  // Tauri должен видеть наш stdout без очистки — иначе теряется
  // диагностика `tauri dev`.
  clearScreen: false,
});
