import { describe, expect, it } from 'vitest';
import { mountHistoryExport } from './history-export';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockResolvedValue: (v: unknown) => void;
  mock: { calls: unknown[][] };
};

describe('mountHistoryExport', () => {
  function buildDom(): { menu: HTMLElement; btn: HTMLElement } {
    const menu = document.createElement('div');
    menu.id = 'export-menu';
    const btn = document.createElement('button');
    btn.id = 'h-export-btn';
    const item = document.createElement('div');
    item.className = 'export-dropdown-item';
    item.dataset.format = 'csv';
    document.body.append(menu, btn, item);
    return { menu, btn };
  }

  it('кнопка экспорта тоглит видимость меню', () => {
    const { menu, btn } = buildDom();
    mountHistoryExport();
    btn.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(menu.classList.contains('visible')).toBe(true);
  });

  it('клик по формату вызывает export_history с форматом', () => {
    buildDom();
    invokeMock.mockResolvedValue(undefined);
    mountHistoryExport();
    document.querySelector<HTMLElement>('.export-dropdown-item')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(invokeMock.mock.calls.at(-1)).toEqual(['export_history', { format: 'csv' }]);
  });

  it('no-op без элементов экспорта', () => {
    expect(() => mountHistoryExport()).not.toThrow();
  });
});
