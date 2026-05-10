// features/settings/mic-device.ts
//
// Обработка активного микрофона + per-device mic_gain. Кнопка ↻
// «Обновить» пере-запрашивает default-устройство у backend и подставляет
// соответствующий override в ползунок. Полезно когда пользователь
// переключает мик в системе (Bluetooth headset, USB, встроенный) без
// перезапуска UI.

import { invoke } from '../../shared/lib/tauri';
import { settingsState, type SettingsConfigSnapshot } from './state';

interface CfgWithMicFields {
  mic_gain_per_device?: Record<string, number>;
  mic_gain?: number;
}

/**
 * Запрашивает у backend имя default-микрофона, обновляет UI
 * («Активный микрофон: …») и подставляет в ползунок gain текущий
 * override (или global mic_gain как fallback). Вызывается при загрузке
 * Settings и по кнопке ↻ Обновить.
 */
export async function refreshMicDevice(cfg: CfgWithMicFields | null): Promise<void> {
  let device = '';
  try {
    device = (await invoke<string>('get_default_input_device_name')) || '';
  } catch (_) { device = ''; }
  settingsState.setActiveMicDevice(device);

  const labelEl = document.getElementById('s-mic-active-device');
  if (labelEl) {
    labelEl.textContent = device || '—';
    labelEl.title = device || '';  // полное имя в tooltip если обрезалось
  }
  const gainEl = document.getElementById('s-mic-gain') as HTMLInputElement | null;
  if (!gainEl) return;
  const overrides = (cfg && cfg.mic_gain_per_device) || {};
  const fallback = (cfg && typeof cfg.mic_gain === 'number') ? cfg.mic_gain : 1.0;
  const current = device && overrides[device] != null
    ? overrides[device]
    : fallback;
  gainEl.value = current.toFixed(1);
}

/**
 * Берёт map из originalConfig и обновляет в нём gain для активного
 * устройства текущим значением ползунка. Возвращает обновлённую копию
 * для save_config.
 *
 * Важно: если новое значение совпадает с тем, что показывалось при
 * загрузке (override либо global fallback) — НЕ создаём override-запись.
 * Иначе change-detection видит diff даже когда пользователь ничего не
 * менял (или вернул значение обратно).
 */
export function collectMicGainOverrides(): Record<string, number> {
  const orig = settingsState.getOriginal();
  const base = (orig && orig.mic_gain_per_device) || {};
  const result: Record<string, number> = { ...base };
  const device = settingsState.getActiveMicDevice();
  if (!device) return result;
  const gainEl = document.getElementById('s-mic-gain') as HTMLInputElement | null;
  if (!gainEl) return result;
  const v = Math.max(0.5, Math.min(10, parseFloat(gainEl.value) || 1.0));
  const displayedAtLoad = base[device] != null
    ? base[device]
    : ((orig && orig.mic_gain) || 1.0);
  if (Math.abs(v - displayedAtLoad) < 0.01) {
    // Значение не изменилось от загрузки — оставляем map как есть.
    // Если override уже был — он сохраняется. Если не было — не создаём.
    return result;
  }
  // Реальное изменение — записываем override для активного устройства.
  result[device] = v;
  return result;
}

// Type-export для использования в settings.ts
export type { SettingsConfigSnapshot };
