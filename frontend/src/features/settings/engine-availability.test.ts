import { beforeEach, describe, expect, it } from 'vitest';
import {
  normalizeTranscriber,
  normalizePreload,
  preloadChangedEngines,
  updatePreloadLocks,
  applyEngineAvailability,
} from './engine-availability';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockImplementation: (fn: (cmd: string) => Promise<unknown>) => void;
};

describe('normalizeTranscriber', () => {
  it('сводит whisper-варианты к whisper, остальное не трогает', () => {
    expect(normalizeTranscriber('whisper-tiny')).toBe('whisper');
    expect(normalizeTranscriber('whisper-large')).toBe('whisper');
    expect(normalizeTranscriber('vosk')).toBe('vosk');
    expect(normalizeTranscriber('gigaam')).toBe('gigaam');
  });
});

describe('normalizePreload', () => {
  it('всегда включает активный транскрайбер, дедуплицирует и сортирует', () => {
    expect(normalizePreload(['gigaam'], 'vosk')).toEqual(['gigaam', 'vosk']);
    expect(normalizePreload(['vosk'], 'vosk')).toEqual(['vosk']);
    expect(normalizePreload(['whisper', 'gigaam'], 'gigaam')).toEqual(['gigaam', 'whisper']);
  });
});

describe('preloadChangedEngines', () => {
  const engines = ['vosk', 'whisper', 'gigaam', 'qwen3asr'];

  it('возвращает только движки с изменившимся членством', () => {
    // Включили qwen3asr → меняется ТОЛЬКО qwen3asr, не vosk (регресс на оранжевую рамку).
    expect(preloadChangedEngines(['gigaam', 'qwen3asr'], ['gigaam'], engines)).toEqual(['qwen3asr']);
    // Выключили whisper.
    expect(preloadChangedEngines(['gigaam'], ['gigaam', 'whisper'], engines)).toEqual(['whisper']);
  });

  it('нет изменений → пустой массив (порядок списков не важен)', () => {
    expect(preloadChangedEngines(['gigaam', 'vosk'], ['vosk', 'gigaam'], engines)).toEqual([]);
  });

  it('несколько изменений сразу', () => {
    expect(preloadChangedEngines(['vosk', 'whisper'], ['gigaam'], engines)).toEqual([
      'vosk',
      'whisper',
      'gigaam',
    ]);
  });
});

describe('updatePreloadLocks', () => {
  beforeEach(() => {
    for (const t of ['vosk', 'whisper', 'gigaam', 'gigaam-rnnt', 'qwen3asr']) {
      const el = document.createElement('div');
      el.id = `s-preload-${t}`;
      document.body.appendChild(el);
    }
  });

  it('блокирует и включает тумблер активного движка, остальные не блокирует', () => {
    updatePreloadLocks('vosk');
    expect(document.getElementById('s-preload-vosk')!.classList.contains('locked')).toBe(true);
    expect(document.getElementById('s-preload-vosk')!.classList.contains('on')).toBe(true);
    expect(document.getElementById('s-preload-whisper')!.classList.contains('locked')).toBe(false);
  });

  it('нормализует whisper-tiny → whisper', () => {
    updatePreloadLocks('whisper-tiny');
    expect(document.getElementById('s-preload-whisper')!.classList.contains('locked')).toBe(true);
  });
});

describe('applyEngineAvailability', () => {
  function makeOption(value: string): HTMLElement {
    const opt = document.createElement('div');
    opt.className = 'custom-select-option';
    opt.dataset.value = value;
    return opt;
  }

  beforeEach(() => {
    const sel = document.createElement('div');
    sel.id = 's-transcriber';
    sel.append(makeOption('vosk'), makeOption('gigaam'), makeOption('qwen3asr'));
    document.body.appendChild(sel);
  });

  it('помечает несобранные и без-моделей движки disabled, установленные оставляет активными', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_compiled_engines') return Promise.resolve(['gigaam', 'qwen3asr']);
      if (cmd === 'get_models') {
        return Promise.resolve([
          { id: 'gigaam-v3', transcriber_type: 'gigaam', installed: true },
          { id: 'qwen3', transcriber_type: 'qwen3asr', installed: false },
        ]);
      }
      return Promise.resolve(null);
    });

    await applyEngineAvailability();

    const opt = (v: string) => document.querySelector<HTMLElement>(`[data-value="${v}"]`)!;
    // vosk не собран → disabled
    expect(opt('vosk').classList.contains('option--disabled')).toBe(true);
    expect(opt('vosk').getAttribute('data-disabled-label')).toContain('engine_unavailable');
    // gigaam собран + модель установлена → активен
    expect(opt('gigaam').classList.contains('option--disabled')).toBe(false);
    // qwen3asr собран, но модель не скачана → disabled с label «нет модели»
    expect(opt('qwen3asr').classList.contains('option--disabled')).toBe(true);
    expect(opt('qwen3asr').getAttribute('data-disabled-label')).toContain('model_not_installed');
  });

  it('при пустом списке compiled (старая сборка) ничего не трогает', async () => {
    invokeMock.mockImplementation(() => Promise.resolve([]));
    await applyEngineAvailability();
    expect(document.querySelector<HTMLElement>('[data-value="vosk"]')!.classList.contains('option--disabled')).toBe(false);
  });
});
