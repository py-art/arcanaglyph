import { beforeEach, describe, expect, it } from 'vitest';
import { collectMicGainOverrides, refreshMicDevice } from './mic-device';
import { settingsState, type SettingsConfigSnapshot } from './state';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockResolvedValue: (v: unknown) => void;
  mockRejectedValue: (v: unknown) => void;
};

// Хелперы внутри внешнего describe (см. hotkey-config.test) — top-level function
// declaration ломает синтез test_scope-узла графа → потеря test→prod рёбер.
describe('mic-device', () => {
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

  function gainInput(value: string): void {
    const el = document.createElement('input');
    el.id = 's-mic-gain';
    el.value = value;
    document.body.appendChild(el);
  }

  describe('collectMicGainOverrides', () => {
    beforeEach(() => {
      settingsState.setOriginal(baseConfig({ mic_gain: 1.0, mic_gain_per_device: {} }));
      settingsState.setActiveMicDevice('USB Mic');
    });

    it('записывает override при реальном изменении ползунка', () => {
      gainInput('2.0');
      expect(collectMicGainOverrides()).toEqual({ 'USB Mic': 2.0 });
    });

    it('не создаёт override если значение совпадает с загруженным fallback', () => {
      gainInput('1.0');
      expect(collectMicGainOverrides()).toEqual({});
    });

    it('без активного устройства возвращает базовую карту без изменений', () => {
      settingsState.setActiveMicDevice('');
      gainInput('5.0');
      expect(collectMicGainOverrides()).toEqual({});
    });
  });

  describe('refreshMicDevice', () => {
    beforeEach(() => {
      const label = document.createElement('div');
      label.id = 's-mic-active-device';
      document.body.appendChild(label);
      gainInput('1.0');
    });

    it('подставляет имя устройства и его per-device gain', async () => {
      invokeMock.mockResolvedValue('USB Mic');
      await refreshMicDevice(baseConfig({ mic_gain_per_device: { 'USB Mic': 3.0 }, mic_gain: 1.0 }));
      expect(document.getElementById('s-mic-active-device')!.textContent).toBe('USB Mic');
      expect((document.getElementById('s-mic-gain') as HTMLInputElement).value).toBe('3.0');
      expect(settingsState.getActiveMicDevice()).toBe('USB Mic');
    });

    it('при ошибке backend показывает прочерк и global fallback', async () => {
      invokeMock.mockRejectedValue(new Error('нет команды'));
      await refreshMicDevice(baseConfig({ mic_gain: 2.0 }));
      expect(document.getElementById('s-mic-active-device')!.textContent).toBe('—');
      expect((document.getElementById('s-mic-gain') as HTMLInputElement).value).toBe('2.0');
    });
  });
});
