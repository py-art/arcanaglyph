// widgets/portal-banner/portal-banner.ts
//
// Однократный баннер на Wayland-сессии без сохранённого XDG RemoteDesktop
// restore_token. Кнопка «Дать разрешение» вызывает `grant_portal_now` —
// popup появляется в expected момент (по клику пользователя), а не при
// первом Ctrl+Ё.

import { invoke } from '../../shared/lib/tauri';
import { showToast } from '../../shared/ui/toast';

const STORAGE_KEY = 'portalBannerDismissed';

export async function mountPortalBanner(): Promise<void> {
  if (localStorage.getItem(STORAGE_KEY) === '1') return;

  let needed = false;
  try {
    needed = await invoke<boolean>('check_portal_grant_needed');
  } catch (_) {
    return;
  }
  if (!needed) return;

  const banner = document.getElementById('portal-banner');
  const grantBtn = document.getElementById('portal-banner-grant');
  const closeBtn = document.getElementById('portal-banner-close');
  if (!banner || !grantBtn || !closeBtn) return;

  banner.classList.add('visible');

  grantBtn.addEventListener('click', async () => {
    (grantBtn as HTMLButtonElement).disabled = true;
    grantBtn.textContent = 'Запрос...';
    try {
      await invoke('grant_portal_now');
      banner.classList.remove('visible');
      localStorage.setItem(STORAGE_KEY, '1');
      showToast('Разрешение получено', 'success', 3000);
    } catch (e) {
      (grantBtn as HTMLButtonElement).disabled = false;
      grantBtn.textContent = 'Повторить';
      showToast('Не удалось получить разрешение: ' + e, 'error', 5000);
    }
  });

  closeBtn.addEventListener('click', () => {
    banner.classList.remove('visible');
    localStorage.setItem(STORAGE_KEY, '1');
  });
}
