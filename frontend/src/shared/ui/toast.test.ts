import { describe, expect, it, vi, afterEach } from 'vitest';
import { showToast } from './toast';

afterEach(() => vi.useRealTimers());

describe('showToast', () => {
  it('выставляет текст и класс по типу', () => {
    const toast = document.createElement('div');
    toast.id = 'toast';
    document.body.appendChild(toast);
    showToast('Готово', 'error', 1000);
    expect(toast.textContent).toBe('Готово');
    expect(toast.className).toBe('toast toast--error visible');
  });

  it('скрывает toast по истечении таймера', () => {
    vi.useFakeTimers();
    const toast = document.createElement('div');
    toast.id = 'toast';
    document.body.appendChild(toast);
    showToast('hi', 'success', 500);
    expect(toast.classList.contains('visible')).toBe(true);
    vi.advanceTimersByTime(500);
    expect(toast.classList.contains('visible')).toBe(false);
  });

  it('no-op без #toast в DOM', () => {
    expect(() => showToast('нет элемента')).not.toThrow();
  });
});
