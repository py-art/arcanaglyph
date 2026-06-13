import { describe, expect, it } from 'vitest';
import {
  setCustomSelectValue,
  initCustomSelects,
  bindCustomSelect,
  closeAllCustomSelects,
  ensureGlobalCloseOnClick,
} from './custom-select';

// Хелпер внутри внешнего describe (см. hotkey-config.test) — top-level function
// declaration ломает синтез test_scope-узла графа → потеря test→prod рёбер.
describe('custom-select', () => {
  function makeSelect(id: string, value = 'a'): HTMLElement {
    const el = document.createElement('div');
    el.className = 'custom-select';
    el.id = id;
    el.dataset.value = value;
    el.innerHTML = `
      <div class="custom-select-trigger"></div>
      <div class="custom-select-options">
        <div class="custom-select-option" data-value="a">Option A</div>
        <div class="custom-select-option" data-value="b">Option B</div>
      </div>`;
    document.body.appendChild(el);
    return el;
  }

  describe('setCustomSelectValue', () => {
  it('обновляет data-value, текст триггера и selected-класс', () => {
    const el = makeSelect('sel');
    setCustomSelectValue('sel', 'b');
    expect(el.dataset.value).toBe('b');
    expect(el.querySelector('.custom-select-trigger')!.textContent).toBe('Option B');
    expect(el.querySelector('[data-value="b"]')!.classList.contains('selected')).toBe(true);
  });

  it('no-op для несуществующего id', () => {
    expect(() => setCustomSelectValue('missing', 'x')).not.toThrow();
  });
});

describe('initCustomSelects', () => {
  it('клик по опции выставляет значение и шлёт change', () => {
    const el = makeSelect('sel');
    initCustomSelects();
    let changed = false;
    el.addEventListener('change', () => { changed = true; });
    el.querySelector<HTMLElement>('[data-value="b"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(el.dataset.value).toBe('b');
    expect(changed).toBe(true);
  });

  it('идемпотентна — повторный init не дублирует обработчики', () => {
    const el = makeSelect('sel');
    initCustomSelects();
    initCustomSelects();
    let count = 0;
    el.addEventListener('change', () => { count += 1; });
    el.querySelector<HTMLElement>('[data-value="b"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(count).toBe(1);
  });
});

describe('bindCustomSelect', () => {
  it('возвращает handle с get/setValue и зовёт onChange', () => {
    makeSelect('sel');
    const changes: string[] = [];
    const handle = bindCustomSelect('sel', { onChange: v => changes.push(v) });
    expect(handle).not.toBeNull();
    handle!.setValue('b');
    expect(handle!.getValue()).toBe('b');
    handle!.el.dispatchEvent(new Event('change'));
    expect(changes).toEqual(['b']);
  });

  it('возвращает null для несуществующего id', () => {
    expect(bindCustomSelect('missing')).toBeNull();
  });
});

describe('closeAllCustomSelects / ensureGlobalCloseOnClick', () => {
  it('закрывает открытый dropdown', () => {
    const el = makeSelect('sel');
    initCustomSelects();
    el.querySelector<HTMLElement>('.custom-select-trigger')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(el.classList.contains('open')).toBe(true);
    closeAllCustomSelects();
    expect(el.classList.contains('open')).toBe(false);
  });

  it('глобальный клик по документу закрывает dropdown (биндится один раз)', () => {
    const el = makeSelect('sel');
    initCustomSelects();
    ensureGlobalCloseOnClick();
    ensureGlobalCloseOnClick();
    el.querySelector<HTMLElement>('.custom-select-trigger')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(el.classList.contains('open')).toBe(true);
    document.body.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    expect(el.classList.contains('open')).toBe(false);
  });
  });
});
