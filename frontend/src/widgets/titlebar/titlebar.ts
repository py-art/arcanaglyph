// widgets/titlebar/titlebar.ts
//
// Кнопки кастомного titlebar: свернуть, развернуть, закрыть.
// Failsafe: если invoke('hide_window') сломан (баг IPC) — fallback
// на webview API (appWindow.minimize / .close), чтобы пользователь
// всегда мог свернуть/закрыть окно.

import { invoke, appWindow } from '../../shared/lib/tauri';

export function mountTitlebar(): void {
  document.getElementById('tb-min')?.addEventListener('click', async () => {
    try { await invoke('hide_window'); }
    catch { try { await appWindow.minimize(); } catch { /* webview API тоже недоступен — свернуть нечем, тихо сдаёмся */ } }
  });
  document.getElementById('tb-max')?.addEventListener('click', async () => {
    try {
      (await appWindow.isMaximized()) ? appWindow.unmaximize() : appWindow.maximize();
    } catch { /* max/unmax — косметика; сбой webview API не критичен */ }
  });
  document.getElementById('tb-close')?.addEventListener('click', async () => {
    try { await invoke('hide_window'); }
    catch { try { await appWindow.close(); } catch { /* webview API тоже недоступен — закрыть нечем, тихо сдаёмся */ } }
  });
}
