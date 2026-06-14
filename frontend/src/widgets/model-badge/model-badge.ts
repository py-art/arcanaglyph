// widgets/model-badge/model-badge.ts
//
// Бейдж активной модели на главной странице (правый верхний угол под
// titlebar). Читает текущий конфиг через `load_config`, маппит filename
// модели в короткое display-имя.

import { invoke } from '../../shared/lib/tauri';
import { MODEL_SHORT_NAMES } from '../../entities/model/types';
import type { CoreConfig } from '../../entities/config/types';

let badgeEl: HTMLElement | null = null;

/** Последний сегмент пути. Сплитит и по `/`, и по `\` — иначе на Windows
 *  (пути с обратными слешами) возвращался бы весь путь целиком. */
function basename(p: string): string {
  return p.split(/[/\\]/).pop() ?? p;
}

export async function updateModelBadge(): Promise<void> {
  if (!badgeEl) return;
  try {
    const cfg = await invoke<CoreConfig>('load_config');
    const modelName =
      cfg.transcriber === 'vosk' ? basename(cfg.model_path)
      : cfg.transcriber === 'whisper' ? basename(cfg.whisper_model_path)
      : cfg.transcriber === 'gigaam' ? basename(cfg.gigaam_model_path)
      : cfg.transcriber === 'qwen3asr' ? basename(cfg.qwen3asr_model_path)
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
