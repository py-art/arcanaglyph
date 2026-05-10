// shared/ui/dropdown.ts
//
// Минимальный helper для toggle-dropdown'ов: клик на trigger открывает
// menu, клик вне закрывает. Применяется к export-menu и подобным.

export interface DropdownOptions {
  /** Trigger button — открывает/закрывает menu. */
  triggerEl: HTMLElement;
  /** Menu container с классом-маркером `.visible`. */
  menuEl: HTMLElement;
}

export function bindDropdown({ triggerEl, menuEl }: DropdownOptions): void {
  triggerEl.addEventListener('click', e => {
    e.stopPropagation();
    menuEl.classList.toggle('visible');
  });
  document.addEventListener('click', () => menuEl.classList.remove('visible'));
  menuEl.addEventListener('click', e => e.stopPropagation());
}
