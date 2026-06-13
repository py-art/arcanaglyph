// src/test-setup.ts
//
// Глобальный setup для vitest. Модули обращаются к window.__TAURI__ и
// window.i18n прямо на уровне импорта (shared/lib/tauri.ts, shared/lib/i18n.ts),
// поэтому стабы должны существовать ДО загрузки тестируемых модулей — setupFiles
// исполняется раньше тест-файлов. invoke/listen — vi.fn(), доступные тестам через
// глобалы __invokeMock / __listenMock для настройки возвратов и проверки вызовов.

import { beforeEach, vi } from 'vitest';

const invokeMock = vi.fn();
const listenMock = vi.fn(async () => () => {});

// Минимальный стаб window.__TAURI__ (core.invoke / event.listen / window.*).
(window as unknown as { __TAURI__: unknown }).__TAURI__ = {
  core: { invoke: invokeMock },
  event: { listen: listenMock },
  window: { getCurrentWindow: () => ({}) },
};

// Стаб i18n: t(path, vars) → детерминированная строка для ассертов + no-op
// setLanguage/applyI18n (их дёргают history/settings-модули на mount).
(window as unknown as { i18n: unknown }).i18n = {
  t: (path: string, vars?: Record<string, unknown>) =>
    vars ? `${path}:${JSON.stringify(vars)}` : path,
  getLanguage: () => 'ru',
  setLanguage: () => {},
  applyI18n: () => {},
};

(globalThis as Record<string, unknown>).__invokeMock = invokeMock;
(globalThis as Record<string, unknown>).__listenMock = listenMock;

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  listenMock.mockResolvedValue(() => {});
  document.body.innerHTML = '';
});
