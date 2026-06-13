// features/settings/engine-availability.ts
//
// Помечает опции в dropdown'е «Движок транскрибации» как disabled
// при двух условиях:
//   (1) движок не включён в cargo-сборку — лейбл «(не собрано)»
//   (2) движок собран, но соответствующей модели нет на диске —
//       лейбл «(нет модели)»
// Опция остаётся видна (для информирования), но click не реагирует
// благодаря CSS pointer-events: none + JS-проверке в опции
// (см. initCustomSelects).

import { i18n } from '../../shared/lib/i18n';
import { getCompiledEngines, getModels, type ModelDescriptor } from '../../entities/model/api';

// Маппит расщеплённое значение dropdown'a ('whisper-tiny' / 'whisper-large')
// в обобщённый тип движка для логики, которая не различает варианты модели.
export function normalizeTranscriber(v: string): string {
  if (v === 'whisper-tiny' || v === 'whisper-large') return 'whisper';
  return v;
}

// Нормализовать preload_models — модель по умолчанию всегда включена
export function normalizePreload(preload: string[], transcriber: string): string[] {
  const set = new Set(preload);
  set.add(transcriber);
  return [...set].sort();
}

// Движки, у которых членство в preload-списке изменилось (cur vs orig).
// Чистая функция для по-тумблерной подсветки изменений: каждый preload-тумблер
// — отдельная строка, и .changed надо вешать только на реально изменившиеся.
export function preloadChangedEngines(cur: string[], orig: string[], engines: readonly string[]): string[] {
  return engines.filter(e => cur.includes(e) !== orig.includes(e));
}

export async function applyEngineAvailability(): Promise<void> {
  let compiled: string[] = [];
  let models: ModelDescriptor[] = [];
  try {
    compiled = await getCompiledEngines();
    models = await getModels();
  } catch (_) { /* старая сборка без команды — все опции активны */ }
  if (!Array.isArray(compiled) || compiled.length === 0) return;
  const sel = document.getElementById('s-transcriber');
  if (!sel) return;
  const notBuiltLabel = i18n.t('settings.engine_unavailable');
  const noModelLabel = i18n.t('settings.model_not_installed');

  // Карта dropdown-value → id модели в реестре. Whisper-варианты явные.
  // Для остальных движков — id первой найденной модели этого transcriber_type.
  const valueToModelId = (value: string): string | null => {
    if (value === 'whisper-tiny') return 'whisper-tiny';
    if (value === 'whisper-large') return 'whisper-large-v3-turbo';
    // Для vosk/gigaam/qwen3asr берём первую модель с подходящим transcriber_type.
    const m = models.find(mm => mm.transcriber_type === value);
    return m ? m.id : null;
  };
  const isInstalled = (modelId: string | null): boolean => {
    if (!modelId) return false;
    const m = models.find(mm => mm.id === modelId);
    return !!(m && m.installed);
  };

  sel.querySelectorAll<HTMLElement>('.custom-select-option').forEach(opt => {
    const dropdownValue = opt.dataset.value || '';
    const engine = normalizeTranscriber(dropdownValue);
    if (!compiled.includes(engine)) {
      // (1) движок не собран
      opt.classList.add('option--disabled');
      opt.setAttribute('data-disabled-label', `(${notBuiltLabel})`);
      return;
    }
    // (2) движок собран — проверяем что модель скачана
    const modelId = valueToModelId(dropdownValue);
    if (isInstalled(modelId)) {
      opt.classList.remove('option--disabled');
      opt.removeAttribute('data-disabled-label');
    } else {
      opt.classList.add('option--disabled');
      opt.setAttribute('data-disabled-label', `(${noModelLabel})`);
    }
  });
}

export function updatePreloadLocks(transcriber: string): void {
  transcriber = normalizeTranscriber(transcriber);
  const voskToggle = document.getElementById('s-preload-vosk');
  const whisperToggle = document.getElementById('s-preload-whisper');
  const gigaamToggle = document.getElementById('s-preload-gigaam');
  const qwen3asrToggle = document.getElementById('s-preload-qwen3asr');
  if (!voskToggle || !whisperToggle || !gigaamToggle || !qwen3asrToggle) return;
  voskToggle.classList.toggle('locked', transcriber === 'vosk');
  whisperToggle.classList.toggle('locked', transcriber === 'whisper');
  gigaamToggle.classList.toggle('locked', transcriber === 'gigaam');
  qwen3asrToggle.classList.toggle('locked', transcriber === 'qwen3asr');
  // Модель по умолчанию всегда ON
  if (transcriber === 'vosk') voskToggle.classList.add('on');
  if (transcriber === 'whisper') whisperToggle.classList.add('on');
  if (transcriber === 'gigaam') gigaamToggle.classList.add('on');
  if (transcriber === 'qwen3asr') qwen3asrToggle.classList.add('on');
}
