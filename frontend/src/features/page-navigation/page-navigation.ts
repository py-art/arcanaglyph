// features/page-navigation/page-navigation.ts
//
// Простой router для single-page app: переключение между main /
// menu / settings / history / about секциями. Использует .visible
// на корневых элементах и data-active на menuBtn.
//
// Pub/sub: feature-модули могут подписаться на смену страницы
// через `subscribePage(listener)` — это позволяет history-странице
// перезагружать список при показе, settings — закрывать save-notice
// при уходе и т. п. Ранее main.ts делал `showPage = function(...)`
// reassignment чтобы добавить логику для history; ESM import-binding
// immutable, поэтому переехали на event-based extension.

import { closeAllCustomSelects } from '../../shared/ui/custom-select';

export type PageName = 'main' | 'menu' | 'settings' | 'history' | 'about';

interface NavRefs {
  menuBtn: HTMLElement;
  menuPage: HTMLElement;
  settingsPage: HTMLElement;
  historyPage: HTMLElement | null;
  aboutPage: HTMLElement | null;
  contentEl: HTMLElement;
}

let refs: NavRefs | null = null;
let currentPage: PageName = 'main';
const pageListeners = new Set<(page: PageName) => void>();

export function initPageNavigation(): void {
  const menuBtn = document.getElementById('menu-btn');
  const menuPage = document.getElementById('menu-page');
  const settingsPage = document.getElementById('settings-page');
  const historyPage = document.getElementById('history-page');
  const aboutPage = document.getElementById('about-page');
  const contentEl = document.querySelector('.content') as HTMLElement | null;
  if (!menuBtn || !menuPage || !settingsPage || !contentEl) return;
  refs = { menuBtn, menuPage, settingsPage, historyPage, aboutPage, contentEl };

  // Кнопка ↩ Назад в titlebar: из main → menu, из любой sub-страницы → menu.
  menuBtn.addEventListener('click', () => {
    if (currentPage === 'main') {
      showPage('menu');
    } else {
      showPage('menu');
    }
  });
}

export function showPage(page: PageName): void {
  if (!refs) return;
  refs.contentEl.style.display = page === 'main' ? '' : 'none';
  refs.menuPage.classList.toggle('visible', page === 'menu');
  refs.settingsPage.classList.toggle('visible', page === 'settings');
  if (refs.historyPage) refs.historyPage.classList.toggle('visible', page === 'history');
  if (refs.aboutPage) refs.aboutPage.classList.toggle('visible', page === 'about');
  refs.menuBtn.classList.toggle('back', page !== 'main');
  // Закрываем все открытые dropdown'ы при смене страницы — иначе portal-менюшка
  // зависнет в body после перехода (legacy main.ts делал closeAllDropdowns()
  // прямо в showPage-reassignment).
  closeAllCustomSelects();
  currentPage = page;
  pageListeners.forEach(l => {
    try { l(page); } catch (_) { /* listener errors не должны ломать nav */ }
  });
}

export function getCurrentPage(): PageName {
  return currentPage;
}

/**
 * Подписка на смену страницы. Возвращает unsubscribe-функцию.
 * Listener вызывается ПОСЛЕ изменения DOM-видимости.
 */
export function subscribePage(listener: (page: PageName) => void): () => void {
  pageListeners.add(listener);
  return () => pageListeners.delete(listener);
}
