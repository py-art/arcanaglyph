// shared/ui/toast.ts
//
// Глобальный toast: короткое всплывающее сообщение внизу экрана.
// CSS живёт в index.html (`.toast` + `.toast--success/error/warning`).

export type ToastType = 'success' | 'error' | 'warning';

let toastTimer: ReturnType<typeof setTimeout> | null = null;

export function showToast(
  msg: string,
  type: ToastType = 'success',
  ms = 2000,
): void {
  const toastEl = document.getElementById('toast');
  if (!toastEl) return;
  toastEl.textContent = msg;
  toastEl.className = `toast toast--${type} visible`;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toastEl.classList.remove('visible'), ms);
}
