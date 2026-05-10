// pages/history/history-page.ts
//
// Страница «История»: список записей с пагинацией, фильтр периода,
// воспроизведение аудио (Web Audio API), переключение между вариантами
// транскрибации, retranscribe, удаление одной/всех записей, экспорт.

import { invoke } from '../../shared/lib/tauri';
import { i18n } from '../../shared/lib/i18n';
import { showToast } from '../../shared/ui/toast';
import { showConfirm } from '../../shared/ui/confirm';
import { initCustomSelects, setCustomSelectValue } from '../../shared/ui/custom-select';
import { showPage, subscribePage } from '../../features/page-navigation/page-navigation';
import { mountHistoryExport } from '../../features/history-export/history-export';
import { renderHistoryEntries, stopCurrentPlayer } from './render';

const HIST_PER_PAGE = 10;
let histPage = 0;

export function mountHistoryPage(): void {
  const historyPageEl = document.getElementById('history-page');
  const historyList = document.getElementById('history-list');
  const historyEmpty = document.getElementById('history-empty');
  const histPagination = document.getElementById('history-pagination');
  const histPrev = document.getElementById('h-prev') as HTMLButtonElement | null;
  const histNext = document.getElementById('h-next') as HTMLButtonElement | null;
  const histPageInfo = document.getElementById('h-page-info');
  const hPeriod = document.getElementById('h-period');
  if (!historyPageEl || !historyList || !historyEmpty || !histPagination
      || !histPrev || !histNext || !histPageInfo || !hPeriod) return;

  // Навигация в меню → история
  document.getElementById('menu-history')?.addEventListener('click', () => {
    showPage('history');
    histPage = 0;
    void loadHistory();
  });
  document.getElementById('menu-about')?.addEventListener('click', () => {
    showPage('about');
  });

  // Закрываем save-notice при уходе со страницы settings (раньше делалось
  // в menuBtn-handler'е, теперь — через subscribePage).
  subscribePage(page => {
    const saveNotice = document.getElementById('save-notice');
    if (saveNotice && page !== 'settings') saveNotice.classList.remove('visible');
  });

  // Фильтр периода (кастомный select — инициализируем)
  initCustomSelects();
  hPeriod.addEventListener('change', async () => {
    histPage = 0;
    await loadHistory();
    try {
      await invoke('set_history_filter', { secs: parseInt((hPeriod as HTMLElement).dataset.value || '0') || 0 });
      showToast(i18n.t('toast.saved'), 'success', 1500);
    } catch (e) {
      showToast(`${i18n.t('toast.save_error')}: ${e}`, 'error', 3000);
    }
  });

  async function loadHistory(): Promise<void> {
    const sinceSecs = parseInt((hPeriod as HTMLElement).dataset.value || '0');
    const result = await invoke<{ entries: any[]; total: number }>('get_history', {
      sinceSecs,
      limit: HIST_PER_PAGE,
      offset: histPage * HIST_PER_PAGE,
    });
    renderHistoryEntries({
      entries: result.entries,
      total: result.total,
      historyList: historyList!,
      historyEmpty: historyEmpty!,
      histPagination: histPagination!,
      histPrev: histPrev!,
      histNext: histNext!,
      histPageInfo: histPageInfo!,
      perPage: HIST_PER_PAGE,
      currentPage: histPage,
      onReload: loadHistory,
    });
  }

  // Восстановить язык интерфейса и период фильтра истории из конфига
  void (async function restoreUiState(): Promise<void> {
    try {
      const cfg = await invoke<any>('load_config');
      if (cfg.language) {
        i18n.setLanguage(cfg.language);
      } else {
        i18n.applyI18n();
      }
      const savedSecs = cfg.history_filter_secs;
      if (savedSecs !== undefined && savedSecs !== null) {
        const opt = hPeriod.querySelector(`[data-value="${savedSecs}"]`);
        if (opt) setCustomSelectValue('h-period', String(savedSecs));
      }
    } catch (_) {
      i18n.applyI18n();
    }
  })();

  histPrev.addEventListener('click', () => {
    if (histPage > 0) {
      histPage--;
      void loadHistory();
      historyPageEl!.scrollTop = 0;
    }
  });
  histNext.addEventListener('click', () => {
    histPage++;
    void loadHistory();
    historyPageEl!.scrollTop = 0;
  });

  document.getElementById('h-clear-btn')?.addEventListener('click', async () => {
    if (await showConfirm(i18n.t('history.confirm_clear'))) {
      await invoke('clear_history');
      await loadHistory();
    }
  });

  // Подвешиваем экспортный dropdown
  mountHistoryExport();

  // Останавливаем плеер при смене страницы (audio source мог продолжать играть).
  subscribePage(page => {
    if (page !== 'history') stopCurrentPlayer();
  });
}
