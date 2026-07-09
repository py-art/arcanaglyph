import { describe, expect, it } from 'vitest';
import { buildSnapshot, resolveTranscriberDropdown } from './load-config';

describe('load-config', () => {
  describe('buildSnapshot', () => {
    it('применяет дефолты для отсутствующих полей', () => {
      const snap = buildSnapshot({ transcriber: 'gigaam' });
      expect(snap.hotkey_pause).toBe('');
      expect(snap.vad_enabled).toBe(true); // undefined !== false → true
      expect(snap.vad_silence_secs).toBe(15);
      expect(snap.remove_fillers).toBe(true);
      expect(snap.mic_gain).toBe(1.0); // не число → 1.0
      expect(snap.mic_gain_per_device).toEqual({});
      expect(snap.retention_hours).toBe(0);
      expect(snap.autostart).toBe(false);
      expect(snap.start_minimized).toBe(false);
      expect(snap.show_widget).toBe(true);
      expect(snap.widget_position).toBe('bottom-center');
      expect(snap.show_tray).toBe(true);
      expect(snap.models_base_dir).toBe('');
    });

    it('сохраняет явно заданные значения и копирует mic_gain_per_device', () => {
      const snap = buildSnapshot({
        transcriber: 'whisper',
        vad_enabled: false,
        vad_silence_secs: 5,
        remove_fillers: false,
        mic_gain: 2.5,
        mic_gain_per_device: { mic1: 1.5 },
        retention_hours: 24,
        autostart: true,
        show_widget: false,
        widget_position: 'top-center',
        show_tray: false,
      });
      expect(snap.vad_enabled).toBe(false);
      expect(snap.vad_silence_secs).toBe(5);
      expect(snap.remove_fillers).toBe(false);
      expect(snap.mic_gain).toBe(2.5);
      expect(snap.mic_gain_per_device).toEqual({ mic1: 1.5 });
      expect(snap.retention_hours).toBe(24);
      expect(snap.autostart).toBe(true);
      expect(snap.show_widget).toBe(false);
      expect(snap.widget_position).toBe('top-center');
      expect(snap.show_tray).toBe(false);
    });
  });

  describe('resolveTranscriberDropdown', () => {
    it('whisper с tiny-путём → whisper-tiny', () => {
      expect(resolveTranscriberDropdown({ transcriber: 'whisper', whisper_model_path: '/m/ggml-tiny.bin' }))
        .toBe('whisper-tiny');
    });

    it('whisper с large-путём → whisper-large', () => {
      expect(resolveTranscriberDropdown({ transcriber: 'whisper', whisper_model_path: '/m/ggml-large-v3-turbo.bin' }))
        .toBe('whisper-large');
    });

    it('whisper без пути → whisper-large (дефолт)', () => {
      expect(resolveTranscriberDropdown({ transcriber: 'whisper' })).toBe('whisper-large');
    });

    it('не-whisper движок возвращается как есть', () => {
      expect(resolveTranscriberDropdown({ transcriber: 'gigaam' })).toBe('gigaam');
      expect(resolveTranscriberDropdown({ transcriber: 'vosk' })).toBe('vosk');
    });
  });
});
