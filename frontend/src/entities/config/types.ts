// entities/config/types.ts
//
// Зеркало `arcanaglyph_core::config::CoreConfig` (Rust). Поля совпадают
// с serialize-форматом (snake_case). Полная синхронизация — на следующей
// итерации (генерация TS-типов из rust-стороны через ts-rs или вручную).

export type TranscriberType = 'vosk' | 'whisper' | 'gigaam' | 'qwen3asr';

export interface CoreConfig {
  transcriber: TranscriberType;
  model_path: string;
  whisper_model_path: string;
  gigaam_model_path: string;
  qwen3asr_model_path: string;
  sample_rate: number;
  max_record_secs: number;
  hotkey: string;
  hotkey_pause?: string;
  auto_type: boolean;
  debug: boolean;
  vad_enabled?: boolean;
  vad_silence_secs?: number;
  remove_fillers?: boolean;
  mic_gain?: number;
  mic_gain_per_device?: Record<string, number>;
  retention_hours?: number;
  autostart?: boolean;
  start_minimized?: boolean;
  show_widget?: boolean;
  widget_position?: string;
  show_tray?: boolean;
  models_base_dir?: string;
  preload_models?: TranscriberType[];
  language?: 'ru' | 'en';
}
