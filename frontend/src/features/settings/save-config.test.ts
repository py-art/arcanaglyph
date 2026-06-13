import { beforeEach, describe, expect, it } from 'vitest';
import { saveConfig, getFormConfig } from './save-config';
import { settingsState, type SettingsConfigSnapshot } from './state';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockImplementation: (fn: (cmd: string) => Promise<unknown>) => void;
  mock: { calls: unknown[][] };
};

describe('save-config', () => {
  function baseConfig(over: Partial<SettingsConfigSnapshot> = {}): SettingsConfigSnapshot {
    return {
      transcriber: 'gigaam', model_path: '', whisper_model_path: '', gigaam_model_path: '',
      qwen3asr_model_path: '', sample_rate: 16000, max_record_secs: 60, hotkey: '', hotkey_pause: '',
      auto_type: false, debug: false, vad_enabled: false, vad_silence_secs: 3, remove_fillers: false,
      mic_gain: 1.0, mic_gain_per_device: {}, retention_hours: 0, autostart: false, start_minimized: false,
      show_widget: true, widget_position: 'bottom-center', show_tray: true, models_base_dir: '',
      preload_models: [], ...over,
    };
  }

  describe('saveConfig', () => {
    it('всегда зовёт save_config; на не-Wayland хоткеи не регистрирует', async () => {
      invokeMock.mockImplementation((cmd: string) =>
        cmd === 'is_wayland' ? Promise.resolve(false) : Promise.resolve(undefined));
      const cfg = baseConfig({ hotkey: 'Super+G' });
      await saveConfig(cfg);
      expect(invokeMock.mock.calls[0]).toEqual(['save_config', { config: cfg }]);
      expect(invokeMock.mock.calls.some(c => c[0] === 'register_gnome_hotkeys')).toBe(false);
    });

    it('на Wayland без конфликтов регистрирует GNOME-хоткеи', async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'is_wayland') return Promise.resolve(true);
        if (cmd === 'check_hotkey_conflict') return Promise.resolve(null);
        return Promise.resolve(undefined);
      });
      await saveConfig(baseConfig({ hotkey: 'Super+G', hotkey_pause: 'Super+P' }));
      expect(invokeMock.mock.calls.some(c => c[0] === 'register_gnome_hotkeys')).toBe(true);
    });
  });

  describe('getFormConfig', () => {
    function select(id: string, value: string): void {
      const el = document.createElement('div');
      el.id = id;
      el.dataset.value = value;
      document.body.appendChild(el);
    }
    function input(id: string, value: string): void {
      const el = document.createElement('input');
      el.id = id;
      el.value = value;
      document.body.appendChild(el);
    }
    function toggle(id: string, on: boolean): void {
      const el = document.createElement('div');
      el.id = id;
      if (on) el.classList.add('on');
      document.body.appendChild(el);
    }

    beforeEach(() => {
      settingsState.setOriginal(baseConfig());
      settingsState.setActiveMicDevice('');
      select('s-transcriber', 'gigaam');
      select('s-sample-rate', '16000');
      select('s-retention', '24');
      select('s-widget-position', 'top-center');
      select('s-language', 'ru');
      select('h-period', '0');
      input('s-models-base-dir', '/models');
      input('s-max-record', '90');
      input('s-vad-silence', '4');
      for (const t of ['s-auto-type', 's-debug', 's-vad-enabled', 's-remove-fillers',
        's-autostart', 's-start-minimized', 's-show-widget', 's-show-tray',
        's-preload-vosk', 's-preload-whisper', 's-preload-gigaam', 's-preload-qwen3asr']) {
        toggle(t, false);
      }
    });

    it('собирает снимок конфига из состояния формы', () => {
      document.getElementById('s-auto-type')!.classList.add('on');
      document.getElementById('s-preload-gigaam')!.classList.add('on');
      const cfg = getFormConfig();
      expect(cfg.transcriber).toBe('gigaam');
      expect(cfg.sample_rate).toBe(16000);
      expect(cfg.max_record_secs).toBe(90);
      expect(cfg.vad_silence_secs).toBe(4);
      expect(cfg.auto_type).toBe(true);
      expect(cfg.debug).toBe(false);
      expect(cfg.widget_position).toBe('top-center');
      expect(cfg.models_base_dir).toBe('/models');
      // активный транскрайбер всегда в preload (normalizePreload)
      expect(cfg.preload_models).toContain('gigaam');
    });

    it('whisper-tiny маппится в transcriber=whisper + путь к ggml-tiny.bin', () => {
      document.getElementById('s-transcriber')!.dataset.value = 'whisper-tiny';
      const cfg = getFormConfig();
      expect(cfg.transcriber).toBe('whisper');
      expect(cfg.whisper_model_path).toBe('/models/ggml-tiny.bin');
    });
  });
});
