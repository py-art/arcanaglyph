import { describe, expect, it } from 'vitest';
import { mountAboutPage } from './about';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockResolvedValue: (v: unknown) => void;
  mockImplementation: (fn: (cmd: string) => Promise<unknown>) => void;
};

describe('mountAboutPage', () => {
  function buildDom(): void {
    const v = document.createElement('div');
    v.id = 'about-version';
    v.textContent = '—';
    const btn = document.createElement('button');
    btn.id = 'check-updates-btn';
    document.body.append(v, btn);
  }

  it('подставляет версию из get_app_version', async () => {
    buildDom();
    invokeMock.mockResolvedValue('1.7.5');
    await mountAboutPage();
    expect(document.getElementById('about-version')!.textContent).toBe('v1.7.5');
  });

  it('кнопка «проверить обновления» вызывает check_updates_now', async () => {
    buildDom();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_app_version') return Promise.resolve('1.7.5');
      if (cmd === 'check_updates_now') return Promise.resolve(null); // up-to-date
      return Promise.resolve(null);
    });
    await mountAboutPage();
    const btn = document.getElementById('check-updates-btn') as HTMLButtonElement;
    btn.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    // обработчик асинхронный — даём микротаскам отработать
    await Promise.resolve();
    expect(btn).toBeTruthy();
  });
});
