// widgets/update-banner/update-banner.ts
//
// Bottom-fixed banner «Доступно обновление X.Y.Z» с тремя действиями:
// Обновить (apply_update — терминал), Что нового (release page),
// × (dismiss). Появляется по событию `update://available` от backend
// (фоновый чекер раз в 24h + cached emit на старте).

import { invoke, listen } from '../../shared/lib/tauri';
import { t } from '../../shared/lib/i18n';
import { showToast } from '../../shared/ui/toast';

interface UpdateInfo {
  latest_version: string;
  release_url: string;
  published_at: string;
}

export function mountUpdateBanner(): void {
  const banner = document.getElementById('update-banner');
  const versionEl = document.getElementById('update-banner-version');
  const applyBtn = document.getElementById('update-banner-apply') as HTMLButtonElement | null;
  const notesBtn = document.getElementById('update-banner-notes');
  const dismissBtn = document.getElementById('update-banner-dismiss');
  if (!banner || !versionEl || !applyBtn || !notesBtn || !dismissBtn) return;

  let currentInfo: UpdateInfo | null = null;

  const show = (info: UpdateInfo): void => {
    currentInfo = info;
    versionEl.textContent = info.latest_version;
    banner.classList.add('visible');
  };

  applyBtn.addEventListener('click', async () => {
    applyBtn.disabled = true;
    try {
      await invoke('apply_update');
      showToast(t('update.toast_terminal_started'), 'success', 5000);
    } catch (e) {
      showToast(t('update.toast_apply_failed', { err: String(e) }), 'error', 6000);
    } finally {
      applyBtn.disabled = false;
    }
  });

  notesBtn.addEventListener('click', async () => {
    if (!currentInfo?.release_url) return;
    try {
      await invoke('open_release_notes', { url: currentInfo.release_url });
    } catch (e) {
      showToast(String(e), 'error', 4000);
    }
  });

  dismissBtn.addEventListener('click', async () => {
    if (!currentInfo) {
      banner.classList.remove('visible');
      return;
    }
    try {
      await invoke('dismiss_update', { version: currentInfo.latest_version });
    } catch (_) {
      // swallow — UI всё равно скрываем
    }
    banner.classList.remove('visible');
  });

  /** Метод-API для manual триггера баннера (используется setupAboutPage). */
  (window as any).__showUpdateBanner = show;

  void listen<UpdateInfo>('update://available', e => show(e.payload));
}
