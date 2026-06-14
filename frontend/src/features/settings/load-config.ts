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

/** Загрузка настроек из бэкенда. */
export async function loadSettings(callbacks: ResetUiCallbacks): Promise<void> {
  const cfg = await invoke<any>('load_config');
  const lang = cfg.language || i18n.getLanguage();
  setCustomSelectValue('s-language', lang);
  await applyEngineAvailability();
  // CPU-фичи нужны для warning-toast'а при выборе тяжёлой модели; берём один раз.
  if (!settingsState.getCpuFeatures()) {
    try {
      const f = await invoke<CpuFeatures>('get_cpu_features');
      settingsState.setCpuFeatures(f);
    } catch (_) {
      settingsState.setCpuFeatures({ avx: true, avx2: true, fma: true });
    }
  }
  // Нормализуем типы чтобы совпадали с getFormConfig()
  const snapshot: SettingsConfigSnapshot = {
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
  settingsState.setOriginal(snapshot);

  // Whisper расщеплён в dropdown'e на Tiny / Large V3 Turbo. Конфиг хранит
  // обобщённый transcriber="whisper" + whisper_model_path, поэтому при загрузке
  // определяем нужный пункт по имени файла модели.
  let dropdownValue = cfg.transcriber;
  if (cfg.transcriber === 'whisper') {
    const path = (cfg.whisper_model_path || '').toLowerCase();
    dropdownValue = path.includes('tiny') ? 'whisper-tiny' : 'whisper-large';
  }
  setCustomSelectValue('s-transcriber', dropdownValue);
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
  // Заполнение mic_gain через async refresh (берёт активное устройство и его override)
  await refreshMicDevice(cfg);
  setCustomSelectValue('s-retention', String(cfg.retention_hours || 0));
  document.getElementById('s-autostart')!.classList.toggle('on', cfg.autostart || false);
  document.getElementById('s-start-minimized')!.classList.toggle('on', cfg.start_minimized || false);
  document.getElementById('s-show-widget')!.classList.toggle('on', cfg.show_widget !== false);
  setWidgetPositionPicker(cfg.widget_position || 'bottom-center');
  document.getElementById('s-show-tray')!.classList.toggle('on', cfg.show_tray !== false);
  const preload: string[] = cfg.preload_models || [];
  document.getElementById('s-preload-vosk')!.classList.toggle('on', preload.includes('vosk') || cfg.transcriber === 'vosk');
  document.getElementById('s-preload-whisper')!.classList.toggle('on', preload.includes('whisper') || cfg.transcriber === 'whisper');
  document.getElementById('s-preload-gigaam')!.classList.toggle('on', preload.includes('gigaam') || cfg.transcriber === 'gigaam');
  document.getElementById('s-preload-gigaam-rnnt')!.classList.toggle('on', preload.includes('gigaam-rnnt') || cfg.transcriber === 'gigaam-rnnt');
  document.getElementById('s-preload-qwen3asr')!.classList.toggle('on', preload.includes('qwen3asr') || cfg.transcriber === 'qwen3asr');
  updatePreloadLocks(cfg.transcriber);

  callbacks.resetSaveButtons();

  await renderModelCards();
}

export { setWidgetPositionPicker };
