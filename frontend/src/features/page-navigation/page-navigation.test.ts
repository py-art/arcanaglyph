import { beforeEach, describe, expect, it } from 'vitest';
import {
  initPageNavigation,
  showPage,
  subscribePage,
  getCurrentPage,
} from './page-navigation';

// Хелперы внутри внешнего describe (см. hotkey-config.test) — top-level function
// declaration ломает синтез test_scope-узла графа → потеря test→prod рёбер.
describe('page-navigation', () => {
  function buildNavDom(): void {
    for (const id of ['menu-btn', 'menu-page', 'settings-page', 'history-page', 'about-page']) {
      const el = document.createElement('div');
      el.id = id;
      document.body.appendChild(el);
    }
    const content = document.createElement('div');
    content.className = 'content';
    document.body.appendChild(content);
  }

  beforeEach(() => {
    buildNavDom();
    initPageNavigation();
    showPage('main');
  });

  it('showPage переключает видимость и метку «назад»', () => {
    showPage('settings');
    expect(document.getElementById('settings-page')!.classList.contains('visible')).toBe(true);
    expect((document.querySelector('.content') as HTMLElement).style.display).toBe('none');
    expect(document.getElementById('menu-btn')!.classList.contains('back')).toBe(true);
    expect(getCurrentPage()).toBe('settings');

    showPage('main');
    expect((document.querySelector('.content') as HTMLElement).style.display).toBe('');
    expect(document.getElementById('menu-btn')!.classList.contains('back')).toBe(false);
  });

  it('subscribePage уведомляет слушателя и отписывает', () => {
    const seen: string[] = [];
    const unsub = subscribePage(p => seen.push(p));
    showPage('history');
    expect(seen).toContain('history');
    unsub();
    showPage('about');
    expect(seen).not.toContain('about');
  });

  it('кнопка «назад»: main→menu, menu→main, settings→menu', () => {
    const btn = document.getElementById('menu-btn')!;
    showPage('main');
    btn.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(getCurrentPage()).toBe('menu');
    btn.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(getCurrentPage()).toBe('main');
    showPage('settings');
    btn.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(getCurrentPage()).toBe('menu');
  });
});
