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
