// features/settings/load-config.ts
//
// Подгружает конфиг с backend и заполняет форму. После этого
// устанавливает settingsState.originalConfig — далее change-detection
// (checkChanges) сможет сравнивать текущую форму с этим snapshot'ом.

import { invoke } from '../../shared/lib/tauri';
import { i18n } from '../../shared/lib/i18n';
import { setCustomSelectValue } from '../../shared/ui/custom-select';
import { applyEngineAvailability, normalizePreload, updatePreloadLocks } from './engine-availability';
import { refreshMicDevice } from './mic-device';
import { setHotkeyValue } from '../hotkey-config/hotkey-config';
import { settingsState, type CpuFeatures, type SettingsConfigSnapshot } from './state';
import { renderModelCards } from '../model-management/model-cards';

// Picker позиции виджета: переставляет .active на нужную точку и обновляет data-value
function setWidgetPositionPicker(pos: string): void {
  const picker = document.getElementById('s-widget-position');
  if (!picker) return;
  picker.dataset.value = pos;
  picker.querySelectorAll<HTMLElement>('.position-picker-dot').forEach(d => {
    d.classList.toggle('active', d.dataset.pos === pos);
  });
}

interface ResetUiCallbacks {
  /** Снять disabled с save/cancel + спрятать save-notice. */
  resetSaveButtons: () => void;
}

/** Нормализует ответ backend в snapshot формы (типы/дефолты совпадают с getFormConfig()).
 *  Чистая функция — тестируема без DOM. */
export function buildSnapshot(cfg: any): SettingsConfigSnapshot {
  return {
    transcriber: cfg.transcriber,
    model_path: cfg.model_path,
    whisper_model_path: cfg.whisper_model_path,
    gigaam_model_path: cfg.gigaam_model_path,
    gigaam_rnnt_model_path: cfg.gigaam_rnnt_model_path,
    qwen3asr_model_path: cfg.qwen3asr_model_path,
    sample_rate: cfg.sample_rate,
    max_record_secs: cfg.max_record_secs,
    hotkey: cfg.hotkey,
    hotkey_pause: cfg.hotkey_pause || '',
    auto_type: cfg.auto_type,
    debug: cfg.debug,
    vad_enabled: cfg.vad_enabled !== false,
    vad_silence_secs: cfg.vad_silence_secs || 15,
    remove_fillers: cfg.remove_fillers !== false,
    mic_gain: typeof cfg.mic_gain === 'number' ? cfg.mic_gain : 1.0,
    mic_gain_per_device: cfg.mic_gain_per_device && typeof cfg.mic_gain_per_device === 'object'
      ? { ...cfg.mic_gain_per_device }
      : {},
    retention_hours: cfg.retention_hours || 0,
    autostart: cfg.autostart || false,
    start_minimized: cfg.start_minimized || false,
    show_widget: cfg.show_widget !== false,
    widget_position: cfg.widget_position || 'bottom-center',
    show_tray: cfg.show_tray !== false,
    models_base_dir: cfg.models_base_dir || '',
    preload_models: normalizePreload(cfg.preload_models || [], cfg.transcriber),
  };
}

/** Пункт dropdown'а транскрайбера. Whisper расщеплён в UI на Tiny / Large V3 Turbo —
 *  конфиг хранит обобщённый transcriber="whisper" + whisper_model_path, поэтому нужный
 *  пункт определяем по имени файла модели. Чистая функция — тестируема без DOM. */
export function resolveTranscriberDropdown(cfg: any): string {
  if (cfg.transcriber !== 'whisper') return cfg.transcriber;
  const path = (cfg.whisper_model_path || '').toLowerCase();
  return path.includes('tiny') ? 'whisper-tiny' : 'whisper-large';
}

/** Разовая загрузка CPU-фич (нужны для warning-toast'а при выборе тяжёлой модели). */
async function loadCpuFeaturesOnce(): Promise<void> {
  if (settingsState.getCpuFeatures()) return;
  try {
    const f = await invoke<CpuFeatures>('get_cpu_features');
    settingsState.setCpuFeatures(f);
  } catch {
    settingsState.setCpuFeatures({ avx: true, avx2: true, fma: true });
  }
}

/** Заполняет основные поля формы: движок, аудио, хоткеи, VAD, флаги. */
function fillGeneralFields(cfg: any): void {
  setCustomSelectValue('s-transcriber', resolveTranscriberDropdown(cfg));
  (document.getElementById('s-models-base-dir') as HTMLInputElement).value = cfg.models_base_dir || '';
  setCustomSelectValue('s-sample-rate', String(cfg.sample_rate));
  (document.getElementById('s-max-record') as HTMLInputElement).value = String(cfg.max_record_secs);
  setHotkeyValue('hk-trigger', cfg.hotkey);
  setHotkeyValue('hk-pause', cfg.hotkey_pause || '');
  document.getElementById('s-auto-type')!.classList.toggle('on', cfg.auto_type);
  document.getElementById('s-debug')!.classList.toggle('on', cfg.debug);
  document.getElementById('s-vad-enabled')!.classList.toggle('on', cfg.vad_enabled !== false);
  (document.getElementById('s-vad-silence') as HTMLInputElement).value = String(cfg.vad_silence_secs || 15);
  document.getElementById('s-remove-fillers')!.classList.toggle('on', cfg.remove_fillers !== false);
}

/** Заполняет поля приватности, автозапуска, виджета и трея. */
function fillPrivacyAndWidgetFields(cfg: any): void {
  setCustomSelectValue('s-retention', String(cfg.retention_hours || 0));
  document.getElementById('s-autostart')!.classList.toggle('on', cfg.autostart || false);
  document.getElementById('s-start-minimized')!.classList.toggle('on', cfg.start_minimized || false);
  document.getElementById('s-show-widget')!.classList.toggle('on', cfg.show_widget !== false);
  setWidgetPositionPicker(cfg.widget_position || 'bottom-center');
  document.getElementById('s-show-tray')!.classList.toggle('on', cfg.show_tray !== false);
}

/** Заполняет preload-тумблеры движков (активный движок всегда включён). */
function fillPreloadToggles(cfg: any): void {
  const preload: string[] = cfg.preload_models || [];
  document.getElementById('s-preload-vosk')!.classList.toggle('on', preload.includes('vosk') || cfg.transcriber === 'vosk');
  document.getElementById('s-preload-whisper')!.classList.toggle('on', preload.includes('whisper') || cfg.transcriber === 'whisper');
  document.getElementById('s-preload-gigaam')!.classList.toggle('on', preload.includes('gigaam') || cfg.transcriber === 'gigaam');
  document.getElementById('s-preload-gigaam-rnnt')!.classList.toggle('on', preload.includes('gigaam-rnnt') || cfg.transcriber === 'gigaam-rnnt');
  document.getElementById('s-preload-qwen3asr')!.classList.toggle('on', preload.includes('qwen3asr') || cfg.transcriber === 'qwen3asr');
  updatePreloadLocks(cfg.transcriber);
}

/** Загрузка настроек из бэкенда. */
export async function loadSettings(callbacks: ResetUiCallbacks): Promise<void> {
  const cfg = await invoke<any>('load_config');
  setCustomSelectValue('s-language', cfg.language || i18n.getLanguage());
  await applyEngineAvailability();
  await loadCpuFeaturesOnce();

  settingsState.setOriginal(buildSnapshot(cfg));

  fillGeneralFields(cfg);
  // Заполнение mic_gain через async refresh (берёт активное устройство и его override)
  await refreshMicDevice(cfg);
  fillPrivacyAndWidgetFields(cfg);
  fillPreloadToggles(cfg);

  callbacks.resetSaveButtons();

  await renderModelCards();
}

export { setWidgetPositionPicker };
