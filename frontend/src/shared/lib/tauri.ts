// shared/lib/tauri.ts
//
// Тонкая обёртка над window.__TAURI__: даём типизированные ссылки на
// `invoke` / `listen` / `appWindow`, чтобы остальной код не дёргал
// global window напрямую и не требовал @ts-ignore.
//
// Полная типизация Tauri-команд — на следующих итерациях
// (нужно сгенерировать TS-types из rust-стороны или прописать вручную
// per-command в entities/*).

const tauri = window.__TAURI__;

export const invoke = tauri.core.invoke;
export const listen = tauri.event.listen;
export const appWindow = tauri.window.getCurrentWindow();

/** Безопасный invoke: пробрасывает ошибку, но не падает на отсутствии команды. */
export async function tryInvoke<T = unknown>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T | null> {
  try {
    return await invoke<T>(cmd, args);
  } catch (_) {
    return null;
  }
}

/**
 * Структурированный payload `engine://error` (соответствует `ApiError` в
 * `crates/arcanaglyph-core/src/error.rs`). `kind` — stable enum для маппинга
 * иконки/CTA, `hint` — опциональная подсказка пользователю «что делать».
 */
export type ApiErrorKind =
  | 'audioDevice'
  | 'audioStream'
  | 'modelLoad'
  | 'diskSpace'
  | 'network'
  | 'inputSimulation'
  | 'engineNotAvailable'
  | 'cancelled'
  | 'internal';

export interface ApiError {
  kind: ApiErrorKind;
  message: string;
  hint?: string;
}

/** Type-guard: похоже ли значение на `ApiError`-payload. */
export function isApiError(value: unknown): value is ApiError {
  return (
    typeof value === 'object' &&
    value !== null &&
    'kind' in value &&
    'message' in value &&
    typeof (value as ApiError).message === 'string'
  );
}

/**
 * Извлекает текст сообщения из произвольной ошибки: ApiError → `message`,
 * Error / string → `String(e)`. Используется в catch'ах: `errorMessage(e)`
 * вместо `${e}`, чтобы не получить `[object Object]` для ApiError.
 */
export function errorMessage(e: unknown): string {
  if (isApiError(e)) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

/** Подсказка пользователю «что делать», если backend её прислал. */
export function errorHint(e: unknown): string | undefined {
  return isApiError(e) ? e.hint : undefined;
}

/**
 * Cancelled — пользователь сам нажал «Стоп», toast'ы для этого kind показывать
 * не нужно. Удобно фильтровать в одном месте: `if (isCancelled(e)) return;`.
 */
export function isCancelled(e: unknown): boolean {
  return isApiError(e) && e.kind === 'cancelled';
}
