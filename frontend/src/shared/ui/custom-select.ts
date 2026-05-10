// shared/ui/custom-select.ts
//
// Кастомный dropdown с portal-positioning (options переносятся в body
// при открытии). Вынесен из main.ts inline-кода: модули могут получить
// явный API setValue/getValue вместо querySelector'инга и
// чтения dataset.value напрямую.
//
// Markup ожидается такой:
//   <div class="custom-select" id="..." data-value="...">
//     <div class="custom-select-trigger">…</div>
//     <div class="custom-select-options">
//       <div class="custom-select-option" data-value="...">…</div>
//       …
//     </div>
//   </div>

export interface CustomSelectHandle {
  /** Установить значение программно (как было setCustomSelect(id, v)). */
  setValue(value: string): void;
  /** Текущее значение (data-value на корневом элементе). */
  getValue(): string;
  /** Корневой DOM-элемент для редких случаев когда нужен прямой доступ. */
  el: HTMLElement;
}

interface CustomSelectInternal extends HTMLElement {
  _initialized?: boolean;
  _close?: () => void;
}

/** Закрыть все открытые custom-select на странице. */
export function closeAllCustomSelects(): void {
  document.querySelectorAll('.custom-select').forEach(s => {
    const el = s as CustomSelectInternal;
    if (el._close) el._close();
  });
}

/**
 * Установить значение custom-select по id (legacy-shape API,
 * соответствует старой setCustomSelect из main.ts).
 */
export function setCustomSelectValue(id: string, value: string): void {
  const sel = document.getElementById(id);
  if (!sel) return;
  applyValueToElement(sel, value);
}

function applyValueToElement(sel: HTMLElement, value: string): void {
  sel.dataset.value = value;
  const opt = sel.querySelector(`[data-value="${value}"]`);
  const trigger = sel.querySelector('.custom-select-trigger');
  if (opt && trigger) {
    trigger.textContent = opt.textContent;
    sel.querySelectorAll('.custom-select-option').forEach(o => o.classList.remove('selected'));
    opt.classList.add('selected');
  }
}

/**
 * Инициализировать все .custom-select на странице. Идемпотентно
 * (повторные вызовы пропускают уже-инициализированные элементы).
 */
export function initCustomSelects(): void {
  document.querySelectorAll<HTMLElement>('.custom-select').forEach(sel => {
    initOneSelect(sel as CustomSelectInternal);
  });
}

function initOneSelect(sel: CustomSelectInternal): void {
  if (sel._initialized) return;
  sel._initialized = true;

  const trigger = sel.querySelector<HTMLElement>('.custom-select-trigger');
  const optionsEl = sel.querySelector<HTMLElement>('.custom-select-options');
  if (!trigger || !optionsEl) return;
  const options = sel.querySelectorAll<HTMLElement>('.custom-select-option');

  const initVal = sel.dataset.value || '';
  const initOpt = sel.querySelector<HTMLElement>(`[data-value="${initVal}"]`);
  if (initOpt) {
    trigger.textContent = initOpt.textContent;
    initOpt.classList.add('selected');
  }

  function openDropdown(): void {
    closeAllCustomSelects();
    // Portal: переносим options в body с fixed позицией
    const rect = trigger!.getBoundingClientRect();
    document.body.appendChild(optionsEl!);
    optionsEl!.classList.add('portal');
    optionsEl!.style.top = rect.bottom + 'px';
    optionsEl!.style.left = rect.left + 'px';
    optionsEl!.style.width = rect.width + 'px';
    sel.classList.add('open');
  }

  function closeDropdown(): void {
    optionsEl!.classList.remove('portal');
    optionsEl!.style.display = '';
    optionsEl!.style.top = '';
    optionsEl!.style.left = '';
    optionsEl!.style.width = '';
    if (optionsEl!.parentNode !== sel) {
      sel.appendChild(optionsEl!);
    }
    sel.classList.remove('open');
  }

  trigger.addEventListener('click', e => {
    e.stopPropagation();
    if (sel.classList.contains('open')) closeDropdown();
    else openDropdown();
  });

  options.forEach(opt => {
    opt.addEventListener('click', e => {
      e.stopPropagation();
      // Опция помечена disabled (движок не включён в текущую сборку,
      // см. applyEngineAvailability). Игнорируем клик, dropdown остаётся
      // открытым, чтобы пользователь увидел доступные пункты.
      if (opt.classList.contains('option--disabled')) return;
      sel.dataset.value = opt.dataset.value || '';
      trigger.textContent = opt.textContent;
      options.forEach(o => o.classList.remove('selected'));
      opt.classList.add('selected');
      closeDropdown();
      sel.dispatchEvent(new Event('change'));
    });
  });

  sel._close = closeDropdown;
}

/**
 * Привязка к одному custom-select по id с typed handle.
 * Удобно для feature-модулей, которые хотят держать ссылку на
 * select и не дёргать DOM-querying каждый раз.
 */
export function bindCustomSelect(
  id: string,
  options: { onChange?: (value: string) => void } = {},
): CustomSelectHandle | null {
  const sel = document.getElementById(id);
  if (!sel) return null;
  initOneSelect(sel as CustomSelectInternal);
  if (options.onChange) {
    sel.addEventListener('change', () => options.onChange!(sel.dataset.value || ''));
  }
  return {
    el: sel,
    setValue(value: string): void {
      applyValueToElement(sel, value);
    },
    getValue(): string {
      return sel.dataset.value || '';
    },
  };
}

// Глобальный close-on-outside-click — регистрируем один раз.
// При повторных импортах этот блок не дублируется (ESM-модули
// исполняются один раз).
let _globalClickBound = false;
export function ensureGlobalCloseOnClick(): void {
  if (_globalClickBound) return;
  _globalClickBound = true;
  document.addEventListener('click', closeAllCustomSelects);
}
