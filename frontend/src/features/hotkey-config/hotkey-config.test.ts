import { beforeEach, describe, expect, it } from 'vitest';
import { getHotkeyValue, setHotkeyValue, initHotkeyComposer } from './hotkey-config';

// Хелперы держим ВНУТРИ внешнего describe (не top-level function declaration) —
// иначе индексатор графа привязывает тест-файл к этому символу вместо синтеза
// test_scope-узла, и test→prod CALLS-рёбра теряются (coverage недосчитывает).
describe('hotkey-config', () => {
  function buildComposer(id: string): HTMLElement {
    const el = document.createElement('div');
    el.id = id;
    el.innerHTML = `
      <div class="hotkey-mod" data-mod="Super"></div>
      <div class="hotkey-mod" data-mod="Control"></div>
      <div class="hotkey-mod" data-mod="Alt"></div>
      <div class="hotkey-mod" data-mod="Shift"></div>
      <input class="hotkey-key-input" />
      <div class="hotkey-preview"></div>
      <button class="hotkey-record-btn"></button>
      <button class="hotkey-clear-btn"></button>`;
    document.body.appendChild(el);
    return el;
  }

  describe('setHotkeyValue / getHotkeyValue', () => {
    beforeEach(() => buildComposer('hk-trigger'));

    it('парсит комбинацию в модификаторы + клавишу и читает обратно', () => {
      setHotkeyValue('hk-trigger', 'Super+Shift+G');
      const el = document.getElementById('hk-trigger')!;
      expect(el.querySelector('[data-mod="Super"]')!.classList.contains('active')).toBe(true);
      expect(el.querySelector('[data-mod="Shift"]')!.classList.contains('active')).toBe(true);
      expect(el.querySelector('[data-mod="Alt"]')!.classList.contains('active')).toBe(false);
      expect((el.querySelector('.hotkey-key-input') as HTMLInputElement).dataset.key).toBe('G');
      expect(el.querySelector('.hotkey-preview')!.getAttribute('data-value')).toBe('Super+Shift+G');
      expect(getHotkeyValue('hk-trigger')).toBe('Super+Shift+G');
    });

    it('пустое значение очищает композер', () => {
      setHotkeyValue('hk-trigger', 'Control+A');
      setHotkeyValue('hk-trigger', '');
      expect(getHotkeyValue('hk-trigger')).toBe('');
    });

    it('getHotkeyValue для несуществующего id возвращает пустую строку', () => {
      expect(getHotkeyValue('missing')).toBe('');
    });
  });

  describe('initHotkeyComposer', () => {
    beforeEach(() => buildComposer('hk-trigger'));

    it('toggle модификатора зовёт onChange и меняет комбинацию', () => {
      setHotkeyValue('hk-trigger', 'G');
      let changes = 0;
      initHotkeyComposer('hk-trigger', () => { changes += 1; });
      document.querySelector<HTMLElement>('[data-mod="Super"]')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      expect(changes).toBe(1);
      expect(getHotkeyValue('hk-trigger')).toBe('Super+G');
    });

    it('кнопка очистки сбрасывает комбинацию и зовёт onChange', () => {
      setHotkeyValue('hk-trigger', 'Alt+K');
      let changes = 0;
      initHotkeyComposer('hk-trigger', () => { changes += 1; });
      document.querySelector<HTMLElement>('.hotkey-clear-btn')!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      expect(changes).toBe(1);
      expect(getHotkeyValue('hk-trigger')).toBe('');
    });
  });
});
