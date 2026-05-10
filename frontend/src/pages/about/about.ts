// pages/about/about.ts
//
// Страница «О приложении»: версия (читается из бинарника через
// get_app_version), кнопка «Проверить обновления» (manual fallback
// для фонового update-checker'а).

import { invoke } from '../../shared/lib/tauri';
import { t } from '../../shared/lib/i18n';
import { showToast } from '../../shared/ui/toast';

interface UpdateInfo {
  latest_version: string;
  release_url: string;
  published_at: string;
}

export async function mountAboutPage(): Promise<void> {
  // Версия — заменяем плейсхолдер «—» на реальную из бинарника.
  try {
    const v = await invoke<string>('get_app_version');
    const versionEl = document.getElementById('about-version');
    if (versionEl) versionEl.textContent = `v${v}`;
  } catch (_) {
    // оставим '—'
  }

  const btn = document.getElementById('check-updates-btn') as HTMLButtonElement | null;
  if (!btn) return;
  btn.addEventListener('click', async () => {
    btn.disabled = true;
    try {
      const info = await invoke<UpdateInfo | null>('check_updates_now');
      if (info) {
        showToast(t('update.toast_available', { v: info.latest_version }), 'success', 4000);
        // Принудительно показываем баннер (backend не эмитит из manual-команды).
        const showBanner = (window as any).__showUpdateBanner as
          | ((info: UpdateInfo) => void)
          | undefined;
        if (showBanner) showBanner(info);
      } else {
        showToast(t('update.toast_up_to_date'), 'success', 3000);
      }
    } catch (err) {
      showToast(t('update.toast_check_failed', { err: String(err) }), 'error', 5000);
    } finally {
      btn.disabled = false;
    }
  });
}
