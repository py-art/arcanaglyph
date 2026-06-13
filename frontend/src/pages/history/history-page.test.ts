import { describe, expect, it } from 'vitest';
import { mountHistoryPage } from './history-page';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockImplementation: (fn: (cmd: string) => Promise<unknown>) => void;
  mock: { calls: unknown[][] };
};

describe('mountHistoryPage', () => {
  function buildDom(): void {
    for (const [id, tag] of [
      ['history-page', 'div'], ['history-list', 'div'], ['history-empty', 'div'],
      ['history-pagination', 'div'], ['h-prev', 'button'], ['h-next', 'button'],
      ['h-page-info', 'div'], ['h-period', 'div'],
    ] as const) {
      const el = document.createElement(tag);
      el.id = id;
      document.body.appendChild(el);
    }
  }

  it('смена периода грузит историю через get_history', async () => {
    buildDom();
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_history') return Promise.resolve({ entries: [], total: 0 });
      return Promise.resolve(null);
    });
    mountHistoryPage();
    const hPeriod = document.getElementById('h-period')!;
    hPeriod.dataset.value = '3600';
    hPeriod.dispatchEvent(new Event('change'));
    await Promise.resolve();
    await Promise.resolve();
    expect(invokeMock.mock.calls.some(c => c[0] === 'get_history')).toBe(true);
  });

  it('no-op без обязательных элементов страницы', () => {
    expect(() => mountHistoryPage()).not.toThrow();
  });
});
