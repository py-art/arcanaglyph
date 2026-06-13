import { defineConfig } from 'vitest/config';

// Юнит-тесты фронтенда: jsdom-окружение + общий setup, который подменяет
// window.__TAURI__ и window.i18n (модули обращаются к ним на уровне импорта).
export default defineConfig({
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
    include: ['src/**/*.test.ts'],
  },
});
