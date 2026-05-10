// features/hotkey-config/hotkey-config.ts
//
// Композер горячих клавиш: модификаторы кнопками + рекордер основной
// клавиши. Super на Wayland не доходит до WebView, поэтому модификаторы
// — кнопки, не keydown. Используется для двух полей: hk-trigger
// (запись/стоп) и hk-pause (пауза).

// Маппинг JS event.code → Tauri-формат
function mapKeyCode(event: KeyboardEvent): string {
  const code = event.code;
  if (code.startsWith('Key')) return code.slice(3);
  if (code.startsWith('Digit')) return code.slice(5);
  if (/^F\d+$/.test(code)) return code;
  const map: Record<string, string> = {
    Space: 'Space', Enter: 'Return', Tab: 'Tab',
    Backspace: 'Backspace', Delete: 'Delete', Insert: 'Insert',
    ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
    Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown',
    Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']',
    Semicolon: ';', Quote: "'", Backquote: '`', Backslash: '\\',
    Comma: ',', Period: '.', Slash: '/',
  };
  return map[code] || event.key;
}

function composeHotkey(composer: HTMLElement): string {
  const mods: string[] = [];
  composer.querySelectorAll<HTMLElement>('.hotkey-mod.active').forEach(btn => {
    if (btn.dataset.mod) mods.push(btn.dataset.mod);
  });
  const keyInput = composer.querySelector<HTMLInputElement>('.hotkey-key-input');
  const key = keyInput?.dataset.key || '';
  if (!key) return '';
  return [...mods, key].join('+');
}

function updateHotkeyPreview(composer: HTMLElement): void {
  const combo = composeHotkey(composer);
  const preview = composer.querySelector<HTMLElement>('.hotkey-preview');
  if (!preview) return;
  preview.dataset.value = combo;
  preview.textContent = combo || '';
}

export function getHotkeyValue(id: string): string {
  const composer = document.getElementById(id);
  return composer ? composeHotkey(composer) : '';
}

export function setHotkeyValue(id: string, value: string): void {
  const composer = document.getElementById(id);
  if (!composer) return;
  // Сбрасываем всё
  composer.querySelectorAll<HTMLElement>('.hotkey-mod').forEach(btn => btn.classList.remove('active'));
  const keyInput = composer.querySelector<HTMLInputElement>('.hotkey-key-input');
  if (!keyInput) return;
  keyInput.value = '';
  keyInput.dataset.key = '';

  if (!value) {
    updateHotkeyPreview(composer);
    return;
  }

  // Парсим "Super+Shift+G" → mods + key
  const parts = value.split('+');
  const modNames = ['Super', 'Control', 'Alt', 'Shift'];
  for (const part of parts) {
    if (modNames.includes(part)) {
      const btn = composer.querySelector<HTMLElement>(`.hotkey-mod[data-mod="${part}"]`);
      if (btn) btn.classList.add('active');
    } else {
      keyInput.value = part;
      keyInput.dataset.key = part;
    }
  }
  updateHotkeyPreview(composer);
}

/**
 * Инициализация одного композера. onChange зовётся когда юзер меняет
 * комбинацию (toggle модификатора, recording новой клавиши, clear).
 */
export function initHotkeyComposer(composerId: string, onChange?: () => void): void {
  const composer = document.getElementById(composerId);
  if (!composer) return;

  // Toggle модификаторов
  composer.querySelectorAll<HTMLElement>('.hotkey-mod').forEach(btn => {
    btn.addEventListener('click', () => {
      btn.classList.toggle('active');
      updateHotkeyPreview(composer);
      onChange?.();
    });
  });

  // Рекордер основной клавиши
  const keyInput = composer.querySelector<HTMLInputElement>('.hotkey-key-input');
  const recordBtn = composer.querySelector<HTMLElement>('.hotkey-record-btn');
  const clearBtn = composer.querySelector<HTMLElement>('.hotkey-clear-btn');
  if (!keyInput || !recordBtn || !clearBtn) return;

  recordBtn.addEventListener('click', () => {
    keyInput.classList.add('recording');
    keyInput.value = 'Нажмите клавишу...';
    keyInput.focus();

    const handler = (e: KeyboardEvent): void => {
      e.preventDefault();
      e.stopPropagation();

      // Пропускаем модификаторы — они управляются кнопками
      if (['Control', 'Alt', 'Shift', 'Meta', 'Super', 'OS'].includes(e.key)) return;

      // Escape — отмена
      if (e.key === 'Escape') {
        keyInput.classList.remove('recording');
        keyInput.value = keyInput.dataset.key || '';
        document.removeEventListener('keydown', handler, true);
        return;
      }

      const mapped = mapKeyCode(e);
      keyInput.dataset.key = mapped;
      keyInput.value = mapped;
      keyInput.classList.remove('recording');
      document.removeEventListener('keydown', handler, true);
      updateHotkeyPreview(composer);
      onChange?.();
    };

    document.addEventListener('keydown', handler, true);
  });

  // Очистка
  clearBtn.addEventListener('click', () => {
    setHotkeyValue(composerId, '');
    onChange?.();
  });
}
