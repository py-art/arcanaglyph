// shared/lib/i18n.ts
//
// Re-export глобального `window.i18n` (определяется в /public/i18n.js,
// загружается синхронно перед main.ts module). Этот wrapper даёт
// модулям type-safe доступ без обращения к window напрямую.
//
// Когда дойдёт черёд переписывать i18n.js на TS — заменим в этом файле
// re-export на полноценную реализацию, остальной код не изменится.

export const i18n = window.i18n;
export const t = (path: string, vars?: Record<string, unknown>): string =>
  i18n.t(path, vars);
