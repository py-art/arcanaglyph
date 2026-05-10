// features/page-navigation/page-navigation.ts
//
// Простой router для single-page app: переключение между main /
// menu / settings / about-style секциями. Использует .visible на
// корневых элементах и data-active на menuBtn.

export type PageName = 'main' | 'menu' | 'settings';

interface NavRefs {
  menuBtn: HTMLElement;
  menuPage: HTMLElement;
  settingsPage: HTMLElement;
  contentEl: HTMLElement;
}

let refs: NavRefs | null = null;
let currentPage: PageName = 'main';

export function initPageNavigation(): void {
  const menuBtn = document.getElementById('menu-btn');
  const menuPage = document.getElementById('menu-page');
  const settingsPage = document.getElementById('settings-page');
  const contentEl = document.querySelector('.content') as HTMLElement | null;
  if (!menuBtn || !menuPage || !settingsPage || !contentEl) return;
  refs = { menuBtn, menuPage, settingsPage, contentEl };
}

export function showPage(page: PageName): void {
  if (!refs) return;
  refs.contentEl.style.display = page === 'main' ? '' : 'none';
  refs.menuPage.classList.toggle('visible', page === 'menu');
  refs.settingsPage.classList.toggle('visible', page === 'settings');
  refs.menuBtn.classList.toggle('back', page !== 'main');
  currentPage = page;
}

export function getCurrentPage(): PageName {
  return currentPage;
}
