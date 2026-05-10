// features/history-export/history-export.ts
//
// Экспорт истории — dropdown с форматами (csv / json / md / txt). Раньше
// логика жила inline в main.ts; вынесена сюда чтобы history-page.ts не
// держал inline-DOM-биндинги. Backend сам пишет файл и возвращает путь
// (см. invoke('export_history')).

import { invoke } from '../../shared/lib/tauri';
import { i18n } from '../../shared/lib/i18n';
import { showToast } from '../../shared/ui/toast';

/**
 * Подключить dropdown «Экспорт» на history-странице. Идемпотентен —
 * безопасно звать один раз при mountHistoryPage.
 */
export function mountHistoryExport(): void {
  const exportMenu = document.getElementById('export-menu');
  const exportBtn = document.getElementById('h-export-btn');
  if (!exportMenu || !exportBtn) return;

  // Открыть/закрыть dropdown по клику на кнопку
  exportBtn.addEventListener('click', e => {
    e.stopPropagation();
    exportMenu.classList.toggle('visible');
  });

  // Закрытие по клику вне dropdown'а
  document.addEventListener('click', () => exportMenu.classList.remove('visible'));
  exportMenu.addEventListener('click', e => e.stopPropagation());

  async function exportHistory(format: string): Promise<void> {
    exportMenu!.classList.remove('visible');
    try {
      await invoke('export_history', { format });
      showToast(i18n.t('toast.file_saved'), 'success', 5000);
    } catch (e) {
      showToast(`${i18n.t('toast.error')}: ${e}`, 'error', 3000);
    }
  }

  document.querySelectorAll<HTMLElement>('.export-dropdown-item').forEach(item => {
    item.addEventListener('click', () => exportHistory(item.dataset.format || ''));
  });
}
