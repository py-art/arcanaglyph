// features/settings/state.ts
//
// Хранилище shared-state для settings-страницы. Раньше в main.ts были
// `let originalConfig = null` и `let cpuFeatures = null` — глобальные
// mutable-переменные, читаемые из десятка handler'ов. Здесь — модуль
// с явным get/set API. Это позволяет:
//   - history-feature читать originalConfig.history_filter_secs без
//     импорта всего settings-блока,
//   - model-management feature брать пути моделей из originalConfig
//     как fallback при пустом input'е (см. getModelPathFromCard).

export interface SettingsConfigSnapshot {
  transcriber: string;
  model_path: string;
  whisper_model_path: string;
  gigaam_model_path: string;
  qwen3asr_model_path: string;
  sample_rate: number;
  max_record_secs: number;
  hotkey: string;
  hotkey_pause: string;
  auto_type: boolean;
  debug: boolean;
  vad_enabled: boolean;
  vad_silence_secs: number;
  remove_fillers: boolean;
  mic_gain: number;
  mic_gain_per_device: Record<string, number>;
  retention_hours: number;
  autostart: boolean;
  start_minimized: boolean;
  show_widget: boolean;
  widget_position: string;
  show_tray: boolean;
  models_base_dir: string;
  preload_models: string[];
  language?: string;
  history_filter_secs?: number;
}

export interface CpuFeatures {
  avx: boolean;
  avx2: boolean;
  fma: boolean;
}

let originalConfig: SettingsConfigSnapshot | null = null;
let cpuFeatures: CpuFeatures | null = null;
let activeMicDevice = '';

export const settingsState = {
  getOriginal(): SettingsConfigSnapshot | null { return originalConfig; },
  setOriginal(cfg: SettingsConfigSnapshot | null): void { originalConfig = cfg; },
  getCpuFeatures(): CpuFeatures | null { return cpuFeatures; },
  setCpuFeatures(f: CpuFeatures): void { cpuFeatures = f; },
  getActiveMicDevice(): string { return activeMicDevice; },
  setActiveMicDevice(name: string): void { activeMicDevice = name; },
};
