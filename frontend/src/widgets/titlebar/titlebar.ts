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
    catch (_) { try { await appWindow.minimize(); } catch (_) {} }
  });
  document.getElementById('tb-max')?.addEventListener('click', async () => {
    try {
      (await appWindow.isMaximized()) ? appWindow.unmaximize() : appWindow.maximize();
    } catch (_) {}
  });
  document.getElementById('tb-close')?.addEventListener('click', async () => {
    try { await invoke('hide_window'); }
    catch (_) { try { await appWindow.close(); } catch (_) {} }
  });
}
