// widgets/model-badge/model-badge.ts
//
// Бейдж активной модели на главной странице (правый верхний угол под
// titlebar). Читает текущий конфиг через `load_config`, маппит filename
// модели в короткое display-имя.

import { invoke } from '../../shared/lib/tauri';
import { MODEL_SHORT_NAMES } from '../../entities/model/types';
import type { CoreConfig } from '../../entities/config/types';

let badgeEl: HTMLElement | null = null;

export async function updateModelBadge(): Promise<void> {
  if (!badgeEl) return;
  try {
    const cfg = await invoke<CoreConfig>('load_config');
    const modelName =
      cfg.transcriber === 'vosk' ? cfg.model_path.split('/').pop()
      : cfg.transcriber === 'whisper' ? cfg.whisper_model_path.split('/').pop()
      : cfg.transcriber === 'gigaam' ? cfg.gigaam_model_path.split('/').pop()
      : cfg.transcriber === 'qwen3asr' ? cfg.qwen3asr_model_path.split('/').pop()
      : cfg.transcriber;
    if (modelName) {
      badgeEl.textContent = MODEL_SHORT_NAMES[modelName] || modelName;
    }
  } catch (_) {
    // тихо — нет engine ещё или конфиг не загрузился
  }
}

export function mountModelBadge(): void {
  badgeEl = document.getElementById('model-badge');
}
