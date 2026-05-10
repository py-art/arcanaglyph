// shared/ui/confirm.ts
//
// Глобальный confirm-modal: возвращает Promise<boolean>.
// Использует общий overlay #modal-overlay с кнопками #modal-confirm /
// #modal-cancel из index.html.

export function showConfirm(
  text: string,
  confirmLabel?: string,
  cancelLabel?: string,
): Promise<boolean> {
  return new Promise(resolve => {
    const overlay = document.getElementById('modal-overlay');
    const confirmEl = document.getElementById('modal-confirm');
    const cancelEl = document.getElementById('modal-cancel');
    const textEl = document.getElementById('modal-text');
    if (!overlay || !confirmEl || !cancelEl || !textEl) {
      resolve(false);
      return;
    }
    const origConfirmText = confirmEl.textContent ?? '';
    const origCancelText = cancelEl.textContent ?? '';
    textEl.textContent = text;
    if (confirmLabel) confirmEl.textContent = confirmLabel;
    if (cancelLabel) cancelEl.textContent = cancelLabel;
    overlay.classList.add('visible');

    const cleanup = () => {
      overlay.classList.remove('visible');
      confirmEl.removeEventListener('click', onConfirm);
      cancelEl.removeEventListener('click', onCancel);
      // Восстанавливаем оригинальные тексты — другие вызовы могут передавать
      // свои custom labels, не должны их утаскивать.
      confirmEl.textContent = origConfirmText;
      cancelEl.textContent = origCancelText;
    };
    const onConfirm = () => { cleanup(); resolve(true); };
    const onCancel = () => { cleanup(); resolve(false); };
    confirmEl.addEventListener('click', onConfirm);
    cancelEl.addEventListener('click', onCancel);
  });
}
