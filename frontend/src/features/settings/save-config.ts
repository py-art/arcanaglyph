// features/settings/save-config.ts
//
// Собирает config из формы и сохраняет на backend. На Wayland
// дополнительно проверяет конфликты горячих клавиш через GNOME-shell
// gsettings и регистрирует комбинации через `register_gnome_hotkeys`.

import { invoke } from '../../shared/lib/tauri';
import { i18n } from '../../shared/lib/i18n';
import { collectMicGainOverrides } from './mic-device';
import { normalizePreload, normalizeTranscriber } from './engine-availability';
import { getModelPathFromCard } from '../model-management/model-cards';
import { getHotkeyValue } from '../hotkey-config/hotkey-config';
import { settingsState, type SettingsConfigSnapshot } from './state';

/**
 * Собрать config из текущего состояния формы.
 * Dropdown даёт расщеплённое значение для whisper ('whisper-tiny' /
 * 'whisper-large'); backend хранит просто transcriber="whisper" +
 * whisper_model_path. Мапим обратно.
 */
export function getFormConfig(): SettingsConfigSnapshot {
  const transcriberSel = document.getElementById('s-transcriber') as HTMLElement;
  const dropdownValue = transcriberSel.dataset.value || '';
  let transcriber = dropdownValue;
  let whisperPathOverride: string | null = null;
  if (dropdownValue === 'whisper-tiny' || dropdownValue === 'whisper-large') {
    transcriber = 'whisper';
    const baseDir = (document.getElementById('s-models-base-dir') as HTMLInputElement).value || '';
    const sep = baseDir.endsWith('/') ? '' : '/';
    whisperPathOverride = baseDir + sep + (dropdownValue === 'whisper-tiny'
      ? 'ggml-tiny.bin'
      : 'ggml-large-v3-turbo.bin');
  }
  const orig = settingsState.getOriginal();
  const sampleRateSel = document.getElementById('s-sample-rate') as HTMLElement;
  const retentionSel = document.getElementById('s-retention') as HTMLElement;
  const widgetPosSel = document.getElementById('s-widget-position') as HTMLElement;
  const langSel = document.getElementById('s-language') as HTMLElement;
  const hPeriodSel = document.getElementById('h-period') as HTMLElement;

  return {
    transcriber,
    model_path: getModelPathFromCard('vosk'),
    whisper_model_path: whisperPathOverride || getModelPathFromCard('whisper'),
    gigaam_model_path: getModelPathFromCard('gigaam'),
    qwen3asr_model_path: getModelPathFromCard('qwen3asr'),
    models_base_dir: (document.getElementById('s-models-base-dir') as HTMLInputElement).value,
    sample_rate: parseInt(sampleRateSel.dataset.value || '0'),
    max_record_secs: parseInt((document.getElementById('s-max-record') as HTMLInputElement).value),
    hotkey: getHotkeyValue('hk-trigger'),
    hotkey_pause: getHotkeyValue('hk-pause'),
    auto_type: document.getElementById('s-auto-type')!.classList.contains('on'),
    debug: document.getElementById('s-debug')!.classList.contains('on'),
    vad_enabled: document.getElementById('s-vad-enabled')!.classList.contains('on'),
    vad_silence_secs: parseInt((document.getElementById('s-vad-silence') as HTMLInputElement).value) || 3,
    remove_fillers: document.getElementById('s-remove-fillers')!.classList.contains('on'),
    // mic_gain (глобальный fallback) и mic_gain_per_device (per-device override).
    // Глобальный mic_gain не редактируется через UI — это просто запоминание
    // последнего значения. Per-device map обновляется в refreshMicDevice/save.
    mic_gain: orig && typeof orig.mic_gain === 'number' ? orig.mic_gain : 1.0,
    mic_gain_per_device: collectMicGainOverrides(),
    retention_hours: parseInt(retentionSel.dataset.value || '0') || 0,
    autostart: document.getElementById('s-autostart')!.classList.contains('on'),
    start_minimized: document.getElementById('s-start-minimized')!.classList.contains('on'),
    show_widget: document.getElementById('s-show-widget')!.classList.contains('on'),
    widget_position: widgetPosSel.dataset.value || 'bottom-center',
    show_tray: document.getElementById('s-show-tray')!.classList.contains('on'),
    preload_models: normalizePreload([
      ...(document.getElementById('s-preload-vosk')!.classList.contains('on') ? ['vosk'] : []),
      ...(document.getElementById('s-preload-whisper')!.classList.contains('on') ? ['whisper'] : []),
      ...(document.getElementById('s-preload-gigaam')!.classList.contains('on') ? ['gigaam'] : []),
      ...(document.getElementById('s-preload-qwen3asr')!.classList.contains('on') ? ['qwen3asr'] : []),
    ], normalizeTranscriber(transcriberSel.dataset.value || '')),
    language: langSel.dataset.value || i18n.getLanguage(),
    history_filter_secs: parseInt(hPeriodSel?.dataset.value || '0') || 0,
  };
}

/**
 * Сохранить конфиг на backend + (на Wayland) зарегистрировать
 * GNOME-хоткеи через gsettings, проверив сначала конфликты.
 *
 * Возвращает true если save завершился успешно (включая случай когда
 * пользователь отказался от регистрации хоткея с конфликтом — config
 * всё равно сохранён).
 */
export async function saveConfig(cfg: SettingsConfigSnapshot): Promise<void> {
  await invoke('save_config', { config: cfg });

  // На Wayland — проверяем конфликты и регистрируем хоткеи через gsettings
  try {
    const wayland = await invoke<boolean>('is_wayland');
    if (wayland) {
      const conflicts: string[] = [];
      if (cfg.hotkey) {
        const c = await invoke<string | null>('check_hotkey_conflict', { hotkey: cfg.hotkey });
        if (c) conflicts.push(i18n.t('hotkey.trigger_conflict', { hotkey: cfg.hotkey, holder: c }));
      }
      if (cfg.hotkey_pause) {
        const c = await invoke<string | null>('check_hotkey_conflict', { hotkey: cfg.hotkey_pause });
        if (c) conflicts.push(i18n.t('hotkey.pause_conflict', { hotkey: cfg.hotkey_pause, holder: c }));
      }
      if (conflicts.length > 0) {
        const msg = i18n.t('hotkey.conflicts_prompt', { list: conflicts.join('\n') });
        if (!confirm(msg)) return;
      }

      await invoke('register_gnome_hotkeys', {
        hotkeyTrigger: cfg.hotkey,
        hotkeyPause: cfg.hotkey_pause,
      });
    }
  } catch (e) {
    console.warn('Не удалось зарегистрировать GNOME хоткеи:', e);
  }
}
