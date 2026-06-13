import { beforeEach, describe, expect, it } from 'vitest';
import { mountPortalBanner } from './portal-banner';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockResolvedValue: (v: unknown) => void;
  mock: { calls: unknown[][] };
};

describe('mountPortalBanner', () => {
  function buildDom(): HTMLElement {
    const banner = document.createElement('div');
    banner.id = 'portal-banner';
    const grant = document.createElement('button');
    grant.id = 'portal-banner-grant';
    const close = document.createElement('button');
    close.id = 'portal-banner-close';
    document.body.append(banner, grant, close);
    return banner;
  }

  beforeEach(() => localStorage.clear());

  it('показывает баннер когда backend сообщает что grant нужен', async () => {
    const banner = buildDom();
    invokeMock.mockResolvedValue(true);
    await mountPortalBanner();
    expect(banner.classList.contains('visible')).toBe(true);
  });

  it('кнопка «дать разрешение» вызывает grant_portal_now', async () => {
    const banner = buildDom();
    invokeMock.mockResolvedValue(true);
    await mountPortalBanner();
    document.getElementById('portal-banner-grant')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await Promise.resolve();
    expect(invokeMock.mock.calls.some(c => c[0] === 'grant_portal_now')).toBe(true);
    expect(banner.classList.contains('visible')).toBe(false);
  });

  it('не показывает баннер если ранее dismissed (localStorage)', async () => {
    localStorage.setItem('portalBannerDismissed', '1');
    const banner = buildDom();
    await mountPortalBanner();
    expect(banner.classList.contains('visible')).toBe(false);
  });
});
