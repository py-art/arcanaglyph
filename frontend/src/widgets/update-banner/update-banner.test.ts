import { describe, expect, it } from 'vitest';
import { mountUpdateBanner } from './update-banner';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockResolvedValue: (v: unknown) => void;
  mock: { calls: unknown[][] };
};

describe('mountUpdateBanner', () => {
  const IDS = [
    'update-banner', 'update-banner-text-available', 'update-banner-text-applying',
    'update-banner-version', 'update-banner-applying-version', 'update-banner-progress',
    'update-banner-actions-available', 'update-banner-actions-applying', 'update-banner-apply',
    'update-banner-notes', 'update-banner-dismiss', 'update-banner-restart',
    'update-banner-dismiss-applying',
  ];

  function buildDom(): void {
    for (const id of IDS) {
      const tag = id.endsWith('apply') || id.endsWith('restart') ? 'button' : 'div';
      const el = document.createElement(tag);
      el.id = id;
      document.body.appendChild(el);
    }
  }

  it('manual __showUpdateBanner показывает баннер с версией', () => {
    buildDom();
    invokeMock.mockResolvedValue(null);
    mountUpdateBanner();
    const show = (window as Window & { __showUpdateBanner?: (i: unknown) => void }).__showUpdateBanner;
    expect(typeof show).toBe('function');
    show!({ latest_version: '2.0.0', release_url: '', published_at: '' });
    expect(document.getElementById('update-banner')!.classList.contains('visible')).toBe(true);
    expect(document.getElementById('update-banner-version')!.textContent).toBe('2.0.0');
  });

  it('кнопка «обновить» вызывает apply_update', () => {
    buildDom();
    invokeMock.mockResolvedValue(null);
    mountUpdateBanner();
    (window as Window & { __showUpdateBanner?: (i: unknown) => void }).__showUpdateBanner!(
      { latest_version: '2.0.0', release_url: '', published_at: '' },
    );
    document.getElementById('update-banner-apply')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(invokeMock.mock.calls.some(c => c[0] === 'apply_update')).toBe(true);
  });

  it('«Перезапустить» заблокирован в applying пока установка не подтверждена', () => {
    buildDom();
    // update_install_ready (и apply_update) → null = установка ещё не завершена
    invokeMock.mockResolvedValue(null);
    mountUpdateBanner();
    (window as Window & { __showUpdateBanner?: (i: unknown) => void }).__showUpdateBanner!(
      { latest_version: '2.0.0', release_url: '', published_at: '' },
    );
    document.getElementById('update-banner-apply')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    const restart = document.getElementById('update-banner-restart') as HTMLButtonElement;
    expect(restart.disabled).toBe(true);
  });

  it('no-op без элементов баннера', () => {
    expect(() => mountUpdateBanner()).not.toThrow();
  });
});
