import { beforeEach, describe, expect, it } from 'vitest';
import { showConfirm } from './confirm';

// Хелпер внутри describe (см. hotkey-config.test) — top-level function declaration
// ломает синтез test_scope-узла графа → потеря test→prod рёбер.
describe('showConfirm', () => {
  function buildModal(): void {
    for (const id of ['modal-overlay', 'modal-confirm', 'modal-cancel', 'modal-text']) {
      const el = document.createElement('div');
      el.id = id;
      document.body.appendChild(el);
    }
  }

  beforeEach(buildModal);

  it('резолвит true по клику «подтвердить» и показывает текст/overlay', async () => {
    const p = showConfirm('Удалить?', 'Да', 'Нет');
    expect(document.getElementById('modal-overlay')!.classList.contains('visible')).toBe(true);
    expect(document.getElementById('modal-text')!.textContent).toBe('Удалить?');
    expect(document.getElementById('modal-confirm')!.textContent).toBe('Да');
    document.getElementById('modal-confirm')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await expect(p).resolves.toBe(true);
    expect(document.getElementById('modal-overlay')!.classList.contains('visible')).toBe(false);
  });

  it('резолвит false по клику «отмена»', async () => {
    const p = showConfirm('Точно?');
    document.getElementById('modal-cancel')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    await expect(p).resolves.toBe(false);
  });

  it('резолвит false если modal-элементов нет в DOM', async () => {
    document.body.innerHTML = '';
    await expect(showConfirm('x')).resolves.toBe(false);
  });
});
