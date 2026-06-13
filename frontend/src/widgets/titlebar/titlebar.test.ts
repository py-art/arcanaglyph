import { describe, expect, it } from 'vitest';
import { mountTitlebar } from './titlebar';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockResolvedValue: (v: unknown) => void;
  mock: { calls: unknown[][] };
};

describe('mountTitlebar', () => {
  function buildButtons(): void {
    for (const id of ['tb-min', 'tb-max', 'tb-close']) {
      const b = document.createElement('button');
      b.id = id;
      document.body.appendChild(b);
    }
  }

  it('кнопка «свернуть» вызывает команду hide_window', () => {
    buildButtons();
    invokeMock.mockResolvedValue(undefined);
    mountTitlebar();
    document.getElementById('tb-min')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(invokeMock.mock.calls.at(-1)).toEqual(['hide_window']);
  });

  it('no-op без элементов titlebar (не падает)', () => {
    expect(() => mountTitlebar()).not.toThrow();
  });
});
