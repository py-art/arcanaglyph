// widgets/update-banner/update-banner.ts
//
// Bottom-fixed banner с двумя UI-режимами:
//
// • available — есть новый релиз. Кнопки «Обновить» / «Что нового» / «×».
// • applying  — пользователь нажал «Обновить»: indeterminate progress
//   + «Перезапустить» + «×». Persistent через UpdateState.applying_version,
//   восстанавливается между запусками приложения до тех пор пока
//   APP_VERSION не догонит applying_version.
//
// События backend'а: `update://available` (есть новый релиз) и
// `update://applying` (восстановление режима при старте). На mount
// дополнительно опрашиваем `get_update_applying` чтобы не зависеть от
// порядка emit'ов.

import { invoke, listen } from '../../shared/lib/tauri';
import { t } from '../../shared/lib/i18n';
import { showToast } from '../../shared/ui/toast';

interface UpdateInfo {
  latest_version: string;
  release_url: string;
  published_at: string;
}

type BannerState =
  | { kind: 'idle' }
  | { kind: 'available'; info: UpdateInfo }
  | { kind: 'applying'; version: string };

export function mountUpdateBanner(): void {
  const banner = document.getElementById('update-banner');
  const textAvailable = document.getElementById('update-banner-text-available');
  const textApplying = document.getElementById('update-banner-text-applying');
  const versionEl = document.getElementById('update-banner-version');
  const applyingVersionEl = document.getElementById('update-banner-applying-version');
  const progress = document.getElementById('update-banner-progress');
  const actionsAvailable = document.getElementById('update-banner-actions-available');
  const actionsApplying = document.getElementById('update-banner-actions-applying');
  const applyBtn = document.getElementById('update-banner-apply') as HTMLButtonElement | null;
  const notesBtn = document.getElementById('update-banner-notes');
  const dismissBtn = document.getElementById('update-banner-dismiss');
  const restartBtn = document.getElementById('update-banner-restart') as HTMLButtonElement | null;
  const dismissApplyingBtn = document.getElementById('update-banner-dismiss-applying');

  if (
    !banner || !textAvailable || !textApplying || !versionEl || !applyingVersionEl ||
    !progress || !actionsAvailable || !actionsApplying ||
    !applyBtn || !notesBtn || !dismissBtn || !restartBtn || !dismissApplyingBtn
  ) return;

  let state: BannerState = { kind: 'idle' };

  const render = (next: BannerState): void => {
    state = next;
    switch (next.kind) {
      case 'idle':
        banner.classList.remove('visible');
        return;
      case 'available':
        versionEl.textContent = next.info.latest_version;
        textAvailable.hidden = false;
        textApplying.hidden = true;
        progress.hidden = true;
        actionsAvailable.hidden = false;
        actionsApplying.hidden = true;
        applyBtn.disabled = false;
        banner.classList.add('visible');
        return;
      case 'applying':
        applyingVersionEl.textContent = next.version;
        textAvailable.hidden = true;
        textApplying.hidden = false;
        progress.hidden = false;
        actionsAvailable.hidden = true;
        actionsApplying.hidden = false;
        restartBtn.disabled = false;
        banner.classList.add('visible');
        return;
    }
  };

  applyBtn.addEventListener('click', async () => {
    if (state.kind !== 'available') return;
    const info = state.info;
    // Переключаем UI до invoke — пользователь видит мгновенный фидбек,
    // даже если терминал ещё стартует.
    render({ kind: 'applying', version: info.latest_version });
    try {
      await invoke('apply_update', { latestVersion: info.latest_version });
      showToast(t('update.toast_terminal_started'), 'success', 5000);
    } catch (e) {
      // Откат: backend сам очистил applying_version при ошибке.
      render({ kind: 'available', info });
      showToast(t('update.toast_apply_failed', { err: String(e) }), 'error', 6000);
    }
  });

  notesBtn.addEventListener('click', async () => {
    if (state.kind !== 'available') return;
    try {
      await invoke('open_release_notes', { url: state.info.release_url });
    } catch (e) {
      showToast(String(e), 'error', 4000);
    }
  });

  dismissBtn.addEventListener('click', async () => {
    if (state.kind !== 'available') {
      render({ kind: 'idle' });
      return;
    }
    const version = state.info.latest_version;
    try {
      await invoke('dismiss_update', { version });
    } catch (_) {
      // swallow — UI всё равно скрываем
    }
    render({ kind: 'idle' });
  });

  restartBtn.addEventListener('click', async () => {
    if (state.kind !== 'applying') return;
    restartBtn.disabled = true;
    try {
      await invoke('restart_app');
      // Если процесс не вышел — оставляем кнопку disabled (Tauri exit
      // отрабатывает асинхронно, фронт ещё живёт ~миллисекунды).
    } catch (e) {
      restartBtn.disabled = false;
      showToast(t('update.toast_restart_failed', { err: String(e) }), 'error', 6000);
    }
  });

  dismissApplyingBtn.addEventListener('click', async () => {
    try {
      await invoke('clear_update_applying');
    } catch (_) {
      // swallow
    }
    render({ kind: 'idle' });
  });

  /** Метод-API для manual триггера баннера (используется setupAboutPage). */
  (window as Window & { __showUpdateBanner?: (info: UpdateInfo) => void }).__showUpdateBanner =
    (info: UpdateInfo): void => {
      // Manual-вызов из About не должен перетирать applying-режим.
      if (state.kind === 'applying') return;
      render({ kind: 'available', info });
    };

  void listen<UpdateInfo>('update://available', e => {
    if (state.kind === 'applying') return;
    render({ kind: 'available', info: e.payload });
  });

  void listen<string>('update://applying', e => {
    render({ kind: 'applying', version: e.payload });
  });

  // На mount — проверяем persistent applying_version. Если есть — сразу
  // в applying-режим (быстрее чем ждать emit от setup hook'а, который
  // может прийти после первого render'а).
  void invoke<string | null>('get_update_applying')
    .then(v => {
      if (v && state.kind === 'idle') {
        render({ kind: 'applying', version: v });
      }
    })
    .catch(() => { /* ignore */ });
}
