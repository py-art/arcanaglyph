import { describe, expect, it } from 'vitest';
import { renderHistoryEntries, stopCurrentPlayer } from './render';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mock: { calls: unknown[][] };
};

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

    it('помечает оригинальную модель звёздочкой (★ + class original)', () => {
      const opts = buildOpts([
        {
          recording: { id: 1, timestamp: 1_700_000_000, duration_secs: 5 },
          transcriptions: [{ model_name: 'GigaAM v3', text: 'привет', transcriber_type: 'gigaam' }],
          audio_exists: true,
        },
      ]);
      renderHistoryEntries(opts);
      const trigger = opts.historyList.querySelector('.custom-select-trigger')!;
      expect(trigger.classList.contains('original')).toBe(true);
      expect(trigger.textContent).toContain('★');
    });

    it('добавляет «нераспознано»-варианты для отсутствующих типов моделей', () => {
      const opts = buildOpts([
        {
          recording: { id: 1, timestamp: 1_700_000_000, duration_secs: 5 },
          transcriptions: [{ model_name: 'GigaAM v3', text: 'привет', transcriber_type: 'gigaam' }],
          audio_exists: true,
        },
      ]);
      renderHistoryEntries(opts);
      // 1 существующая (gigaam) + 4 нераспознанных (vosk/whisper/gigaam-rnnt/qwen3asr)
      const options = opts.historyList.querySelectorAll('.custom-select-option');
      expect(options.length).toBe(5);
      const vosk = opts.historyList.querySelector('.custom-select-option[data-type="vosk"]')!;
      expect(vosk.getAttribute('data-has-text')).toBe('false');
      expect(vosk.textContent).toContain('history.not_recognized');
    });

    it('показывает кнопку «Распознать» когда текста нет, но есть аудио', () => {
      const opts = buildOpts([
        {
          recording: { id: 7, timestamp: 1_700_000_000, duration_secs: 5 },
          transcriptions: [],
          audio_exists: true,
        },
      ]);
      renderHistoryEntries(opts);
      const btn = opts.historyList.querySelector('.hist-retranscribe-btn')!;
      expect(btn).not.toBeNull();
      expect(btn.getAttribute('data-rid')).toBe('7');
    });

    it('показывает «аудио удалено» когда нет ни текста, ни аудио', () => {
      const opts = buildOpts([
        {
          recording: { id: 1, timestamp: 1_700_000_000, duration_secs: 5 },
          transcriptions: [],
          audio_exists: false,
        },
      ]);
      renderHistoryEntries(opts);
      expect(opts.historyList.querySelector('[data-i18n="history.audio_deleted"]')).not.toBeNull();
      expect(opts.historyList.querySelector('.hist-retranscribe-btn')).toBeNull();
    });

    it('переключение модели в дропдауне перерисовывает контент', () => {
      const opts = buildOpts([
        {
          recording: { id: 1, timestamp: 1_700_000_000, duration_secs: 5 },
          transcriptions: [{ model_name: 'GigaAM v3', text: 'привет', transcriber_type: 'gigaam' }],
          audio_exists: true,
        },
      ]);
      renderHistoryEntries(opts);
      // Изначально — текст оригинальной модели
      expect(opts.historyList.querySelector('.hist-text')!.textContent).toBe('привет');
      // Переключаемся на нераспознанный Vosk → должна появиться кнопка «Распознать»
      const selectWrap = opts.historyList.querySelector<HTMLElement>('.custom-select')!;
      selectWrap.dataset.value = 'Vosk history.not_recognized';
      selectWrap.dispatchEvent(new Event('change'));
      expect(opts.historyList.querySelector('.hist-retranscribe-btn')).not.toBeNull();
      expect(opts.historyList.querySelector('.hist-text')).toBeNull();
    });

    it('удаление записи зовёт delete_history_entry и onReload', async () => {
      const opts = buildOpts([
        {
          recording: { id: 42, timestamp: 1_700_000_000, duration_secs: 5 },
          transcriptions: [{ model_name: 'GigaAM v3', text: 'привет', transcriber_type: 'gigaam' }],
          audio_exists: true,
        },
      ]);
      let reloaded = 0;
      opts.onReload = async () => { reloaded += 1; };
      renderHistoryEntries(opts);
      const delBtn = opts.historyList.querySelector<HTMLButtonElement>('.hist-btn.delete')!;
      delBtn.click();
      await Promise.resolve();
      await Promise.resolve();
      const call = invokeMock.mock.calls.find(c => c[0] === 'delete_history_entry');
      expect(call).toBeTruthy();
      expect((call![1] as { recordingId: number }).recordingId).toBe(42);
      expect(reloaded).toBe(1);
    });
  });

  describe('stopCurrentPlayer', () => {
    it('no-op когда нет активного плеера', () => {
      expect(() => stopCurrentPlayer()).not.toThrow();
    });
  });
});
