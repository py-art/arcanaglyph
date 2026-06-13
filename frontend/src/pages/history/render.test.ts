import { describe, expect, it } from 'vitest';
import { renderHistoryEntries, stopCurrentPlayer } from './render';

// Хелпер и тип внутри внешнего describe (см. hotkey-config.test) — top-level
// function/interface ломает синтез test_scope-узла графа → потеря test→prod рёбер.
describe('history/render', () => {
  interface Entry {
    recording: { id: number; timestamp: number; duration_secs: number };
    transcriptions: Array<{ model_name: string; text: string; transcriber_type: string }>;
    audio_exists: boolean;
  }

  function buildOpts(entries: Entry[]) {
    const mk = (id: string, tag = 'div') => {
      const el = document.createElement(tag);
      el.id = id;
      document.body.appendChild(el);
      return el;
    };
    return {
      entries,
      total: entries.length,
      historyList: mk('history-list'),
      historyEmpty: mk('history-empty'),
      histPagination: mk('hist-pagination'),
      histPrev: mk('hist-prev', 'button') as HTMLButtonElement,
      histNext: mk('hist-next', 'button') as HTMLButtonElement,
      histPageInfo: mk('hist-page-info'),
      perPage: 20,
      currentPage: 0,
      onReload: async () => {},
    };
  }

  describe('renderHistoryEntries', () => {
    it('пустой список показывает «пусто» и прячет пагинацию', () => {
      const opts = buildOpts([]);
      renderHistoryEntries(opts);
      expect(opts.historyEmpty.style.display).toBe('block');
      expect(opts.histPagination.classList.contains('visible')).toBe(false);
      expect(opts.historyList.children.length).toBe(0);
    });

    it('рендерит карточку записи с дропдауном моделей', () => {
      const opts = buildOpts([
        {
          recording: { id: 1, timestamp: 1_700_000_000, duration_secs: 5 },
          transcriptions: [{ model_name: 'GigaAM v3', text: 'привет', transcriber_type: 'gigaam' }],
          audio_exists: true,
        },
      ]);
      renderHistoryEntries(opts);
      expect(opts.historyEmpty.style.display).toBe('none');
      expect(opts.historyList.querySelectorAll('.hist-entry').length).toBe(1);
      expect(opts.historyList.querySelector('.hist-text')!.textContent).toBe('привет');
    });

    it('показывает пагинацию когда записей больше одной страницы', () => {
      const opts = buildOpts([
        {
          recording: { id: 1, timestamp: 1_700_000_000, duration_secs: 5 },
          transcriptions: [],
          audio_exists: false,
        },
      ]);
      opts.total = 40; // 2 страницы при perPage=20
      renderHistoryEntries(opts);
      expect(opts.histPagination.classList.contains('visible')).toBe(true);
    });
  });

  describe('stopCurrentPlayer', () => {
    it('no-op когда нет активного плеера', () => {
      expect(() => stopCurrentPlayer()).not.toThrow();
    });
  });
});
