import { beforeEach, describe, expect, it } from 'vitest';
import { getModelPathFromCard, renderModelCards, initModelCardsHandlers } from './model-cards';
import { settingsState, type SettingsConfigSnapshot } from '../settings/state';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockResolvedValue: (v: unknown) => void;
};

// Хелперы внутри внешнего describe (см. hotkey-config.test) — top-level function
// declaration ломает синтез test_scope-узла графа → потеря test→prod рёбер.
describe('model-cards', () => {
  function baseConfig(over: Partial<SettingsConfigSnapshot> = {}): SettingsConfigSnapshot {
    return {
      transcriber: 'gigaam', model_path: '', whisper_model_path: '', gigaam_model_path: '',
      gigaam_rnnt_model_path: '', qwen3asr_model_path: '', sample_rate: 16000, max_record_secs: 60, hotkey: '', hotkey_pause: '',
      auto_type: false, debug: false, vad_enabled: false, vad_silence_secs: 3, remove_fillers: false,
      mic_gain: 1.0, mic_gain_per_device: {}, retention_hours: 0, autostart: false, start_minimized: false,
      show_widget: true, widget_position: 'bottom-center', show_tray: true, models_base_dir: '',
      preload_models: [], ...over,
    };
  }

  function cardInput(type: string, value: string): void {
    const el = document.createElement('input');
    el.className = 'model-path-input';
    el.dataset.type = type;
    el.value = value;
    document.body.appendChild(el);
  }

  describe('getModelPathFromCard', () => {
    beforeEach(() => settingsState.setOriginal(null));

    it('берёт значение из input карточки если оно есть', () => {
      cardInput('vosk', '/models/vosk-ru');
      expect(getModelPathFromCard('vosk')).toBe('/models/vosk-ru');
    });

    it('при пустом input откатывается на путь из originalConfig по типу', () => {
      cardInput('whisper', '   ');
      settingsState.setOriginal(baseConfig({ whisper_model_path: '/orig/whisper.bin' }));
      expect(getModelPathFromCard('whisper')).toBe('/orig/whisper.bin');
    });

    it('без карточки и без original возвращает пустую строку', () => {
      expect(getModelPathFromCard('gigaam')).toBe('');
    });
  });

  describe('renderModelCards', () => {
    it('рендерит карточку на каждую модель из get_models', async () => {
      const container = document.createElement('div');
      container.id = 'model-cards-container';
      document.body.appendChild(container);
      invokeMock.mockResolvedValue([
        {
          id: 'gigaam-v3', display_name: 'GigaAM v3', description: 'desc', size: '225 MB',
          download_url: '', default_filename: 'gigaam-v3', transcriber_type: 'gigaam', installed: true,
        },
      ]);
      await renderModelCards();
      expect(container.querySelectorAll('.model-card').length).toBe(1);
      expect(container.querySelector('.model-path-input')!.getAttribute('data-type')).toBe('gigaam');
    });

    it('no-op без контейнера', async () => {
      await expect(renderModelCards()).resolves.toBeUndefined();
    });
  });

  describe('initModelCardsHandlers', () => {
    it('регистрируется без ошибок (подписки + колбэки)', () => {
      expect(() => initModelCardsHandlers({ onChange: () => {}, reloadSettings: async () => {} })).not.toThrow();
    });
  });
});
