// features/model-management/model-cards.ts
//
// Карточки моделей в табе «Модели» settings-страницы. Содержит:
//   - render карточек (список + статус installed/missing/unavailable)
//   - кнопка «Скачать» с прогрессом (download_model + listen download://*)
//   - кнопка «Удалить» с подтверждением (delete_model)
//   - выбор директории для модели (Tauri dialog)
//   - getModelPathFromCard — fallback в settingsState.originalConfig
//
// Прогресс активных загрузок хранится вне DOM (Map activeDownloads),
// чтобы пережить re-render после `loadSettings()` (например, юзер удалил
// одну модель пока другая качается).

import { invoke, listen } from '../../shared/lib/tauri';
import { i18n } from '../../shared/lib/i18n';
import { showToast } from '../../shared/ui/toast';
import { downloadModel, deleteModel, getModels, type ModelDescriptor } from '../../entities/model/api';
import { settingsState } from '../settings/state';

interface ProgressSnapshot {
  percent: number;
  file_idx: number;
  total_files: number;
  file?: string;
}

const activeDownloads = new Map<string, ProgressSnapshot>();

// Колбэк для триггера checkChanges из родительской settings-feature.
// Регистрируется через initModelCards({ onChange }).
let onChangeCallback: (() => void) | null = null;
// Колбэк для триггера полной перезагрузки settings (после download/delete).
let reloadSettingsCallback: (() => Promise<void>) | null = null;

export interface ModelCardsOptions {
  onChange: () => void;
  reloadSettings: () => Promise<void>;
}

export function initModelCardsHandlers(opts: ModelCardsOptions): void {
  onChangeCallback = opts.onChange;
  reloadSettingsCallback = opts.reloadSettings;

  // Прогресс скачивания модели — обновляем глобальный state + DOM.
  void listen<{ model_id: string; percent: number; file_idx: number; total_files: number; file?: string }>(
    'download://progress',
    ev => {
      const { model_id, percent, file_idx, total_files, file } = ev.payload;
      activeDownloads.set(model_id, { percent, file_idx, total_files, file });
      applyProgressToCard(model_id);
    },
  );

  // Backend начал распаковку архива (Vosk качается одним .zip ~1.8 ГБ → 2.6 ГБ).
  // Без этого события UI бы видел застывший 100% на 30-90с — выглядит как зависание.
  // Меняем текст кнопки и прогресс-метку на «Распаковка…»; `loadSettings()` в
  // download://complete потом перерисует карточку начисто.
  void listen<{ model_id?: string }>('download://extracting', ev => {
    const model_id = ev?.payload?.model_id;
    if (!model_id) return;
    const card = document.querySelector(`.model-card[data-model-id="${model_id}"]`);
    if (!card) return;
    const btn = card.querySelector('.model-download-btn');
    if (btn) btn.textContent = i18n.t('model.extracting');
    const text = card.querySelector('.model-progress-text');
    if (text) text.textContent = i18n.t('model.extracting');
  });

  // Завершение скачивания. Backend сам обновляет config-путь (см. download_model
  // в Rust), поэтому делаем полный loadSettings — он перерисует карточки (с актуальным
  // путем в input'е), пере-применит availability к dropdown'у (Whisper Tiny больше
  // не «(нет модели)»), обновит originalConfig для save-логики.
  void listen<{ model_id?: string }>('download://complete', ev => {
    if (ev && ev.payload && ev.payload.model_id) {
      activeDownloads.delete(ev.payload.model_id);
    }
    if (reloadSettingsCallback) reloadSettingsCallback().catch(() => {});
  });
}

// Применить состояние прогресса к карточке (после re-render или нового события).
function applyProgressToCard(modelId: string): void {
  const card = document.querySelector(`.model-card[data-model-id="${modelId}"]`);
  if (!card) return;
  const prog = activeDownloads.get(modelId);
  if (!prog) return;
  const wrap = card.querySelector<HTMLElement>('.model-progress');
  const fill = card.querySelector<HTMLElement>('.model-progress-fill');
  const text = card.querySelector<HTMLElement>('.model-progress-text');
  if (wrap) wrap.style.display = 'flex';
  if (fill) fill.style.width = prog.percent + '%';
  if (text) {
    text.textContent = prog.total_files > 1
      ? `[${prog.file_idx + 1}/${prog.total_files}] ${prog.percent}%`
      : `${prog.percent}%`;
  }
  // На время загрузки прячем кнопку «Скачать» (если в свежем рендере она есть).
  const btn = card.querySelector<HTMLButtonElement>('.model-download-btn');
  if (btn) {
    btn.disabled = true;
    btn.textContent = i18n.t('model.downloading');
  }
}

/** Рендер карточек моделей на табе «Модели». */
export async function renderModelCards(): Promise<void> {
  const container = document.getElementById('model-cards-container');
  if (!container) return;
  try {
    const models: ModelDescriptor[] = await getModels();
    container.innerHTML = '';
    for (const m of models) {
      const card = document.createElement('div');
      card.className = 'model-card' + (m.available === false ? ' unavailable' : '');
      card.dataset.modelId = m.id;
      // Для недоступных движков в шапке через ::after подставляется метка.
      const unavailLabel = m.available === false ? i18n.t('model.not_available') : '';
      // Статус карточки: установлена / не установлена / не доступна (backend не собран)
      let statusCls: string;
      let statusKey: string;
      let statusText: string;
      if (m.available === false) {
        statusCls = 'unavailable'; statusKey = 'model.not_available'; statusText = unavailLabel;
      } else if (m.installed) {
        statusCls = 'found'; statusKey = 'model.installed'; statusText = i18n.t('model.installed');
      } else {
        statusCls = 'missing'; statusKey = 'model.missing'; statusText = i18n.t('model.missing');
      }
      const showDownload = m.available !== false && !m.installed;
      const nameAttr = m.available === false ? ` data-unavailable-label="${unavailLabel}"` : '';
      const inputDisabled = m.available === false ? 'disabled' : 'readonly';
      const pickDisabled = m.available === false ? 'disabled' : '';
      card.innerHTML = `
        <div class="model-card-header">
          <span class="model-card-name"${nameAttr}>${m.display_name}</span>
          <span class="model-card-size">${m.size}</span>
        </div>
        <div class="model-card-desc">${m.description}</div>
        <div class="model-card-path">
          <input type="text" class="model-path-input" data-type="${m.transcriber_type}" value="${m.path || ''}" ${inputDisabled} data-i18n-placeholder="model.pick_placeholder" placeholder="${i18n.t('model.pick_placeholder')}" title="${m.path || ''}" />
          <button class="path-pick-btn model-path-pick" data-type="${m.transcriber_type}" data-i18n-title="model.pick_dir_title" title="${i18n.t('model.pick_dir_title')}" ${pickDisabled}>📁</button>
        </div>
        <div class="model-card-footer">
          <span class="model-card-status ${statusCls}" data-i18n="${statusKey}">
            ${statusText}
          </span>
          <div class="model-card-actions">
            ${showDownload ? `<button class="model-download-btn" data-id="${m.id}" data-url="${m.download_url}" data-filename="${m.default_filename}" data-type="${m.transcriber_type}" data-i18n="model.download">${i18n.t('model.download')}</button>` : ''}
            ${m.installed ? `<button class="model-delete-btn" data-id="${m.id}" data-name="${m.display_name}" data-size="${m.size}" data-path="${m.path || ''}" data-i18n="model.delete">${i18n.t('model.delete')}</button>` : ''}
          </div>
        </div>
        <div class="model-progress" style="display:none;">
          <div class="model-progress-bar"><div class="model-progress-fill"></div></div>
          <span class="model-progress-text">0%</span>
        </div>
      `;
      container.appendChild(card);
    }
    // Обработчики кнопок выбора директории в карточках
    container.querySelectorAll<HTMLElement>('.model-path-pick').forEach(btn => {
      btn.addEventListener('click', async () => {
        const input = btn.parentElement?.querySelector<HTMLInputElement>('.model-path-input');
        if (!input) return;
        const baseInput = document.getElementById('s-models-base-dir') as HTMLInputElement | null;
        const currentPath = input.value || baseInput?.value || '';
        const selected = await (window as any).__TAURI__.dialog.open({
          directory: true,
          title: i18n.t('model.pick_dir_title'),
          defaultPath: currentPath,
        });
        if (selected) {
          input.value = selected;
          onChangeCallback?.();
        }
      });
    });
    // Отслеживание изменений в путях карточек
    container.querySelectorAll<HTMLInputElement>('.model-path-input').forEach(input => {
      input.addEventListener('input', () => onChangeCallback?.());
    });
    // Обработчики кнопок «Скачать»
    container.querySelectorAll<HTMLButtonElement>('.model-download-btn').forEach(btn => {
      btn.addEventListener('click', async () => {
        const modelId = btn.dataset.id || '';
        const url = btn.dataset.url || '';
        const filename = btn.dataset.filename || '';
        const ttype = btn.dataset.type || '';

        // Определяем директорию скачивания из пути в карточке
        const card = btn.closest('.model-card');
        const pathInput = card?.querySelector<HTMLInputElement>('.model-path-input');
        const baseInput = document.getElementById('s-models-base-dir') as HTMLInputElement | null;
        let destDir = pathInput ? pathInput.value : '';
        if (!destDir) destDir = (baseInput?.value || '') + '/' + filename;
        // Для whisper путь — к файлу, берём директорию
        if (ttype === 'whisper') {
          destDir = destDir.substring(0, destDir.lastIndexOf('/') + 1) || destDir;
        }

        btn.disabled = true;
        btn.textContent = i18n.t('model.downloading');
        btn.dataset.i18n = 'model.downloading';
        const progress = card?.querySelector<HTMLElement>('.model-progress');
        if (progress) progress.style.display = 'flex';

        try {
          await downloadModel(modelId, url, destDir);
        } catch (e) {
          btn.disabled = false;
          btn.textContent = i18n.t('model.download');
          btn.dataset.i18n = 'model.download';
          if (progress) progress.style.display = 'none';
          alert(i18n.t('model.download_error') + ': ' + e);
        }
      });
    });
    // Обработчики кнопок «Удалить» — удаляют файлы модели физически.
    // Конфиг при этом не трогаем (whisper_model_path и т.п. остаются),
    // но статус в карточке обновится на «Не найдена», и UI предложит скачать заново.
    container.querySelectorAll<HTMLButtonElement>('.model-delete-btn').forEach(btn => {
      btn.addEventListener('click', async () => {
        const modelId = btn.dataset.id || '';
        const path = btn.dataset.path || '';
        const name = btn.dataset.name || '';
        const size = btn.dataset.size || '';
        if (!path) return;
        const confirmMsg = i18n.t('model.delete_confirm')
          .replace('{name}', name)
          .replace('{size}', size);
        if (!confirm(confirmMsg)) return;
        btn.disabled = true;
        try {
          await deleteModel(modelId, path);
          showToast(i18n.t('model.deleted'), 'success', 1500);
          // Backend очистил config-путь если он совпадал с удалённым → переподтянем
          // originalConfig и dropdown'ы (иначе fallback в getModelPathFromCard
          // вернёт старый путь и save восстановит). Полный loadSettings также
          // перерисовывает model-cards с пустым input для удалённой.
          if (reloadSettingsCallback) await reloadSettingsCallback();
        } catch (e) {
          btn.disabled = false;
          showToast(i18n.t('model.delete_error') + ': ' + e, 'error', 3000);
        }
      });
    });
    // Re-render может произойти посреди активной загрузки (например, юзер
    // нажал «Удалить» на одной модели пока другая качается). Восстанавливаем
    // прогресс-бары для всех всё-ещё-активных загрузок — они нашлись в свежей
    // DOM по data-model-id и получают актуальный процент из activeDownloads.
    for (const [modelId] of activeDownloads) applyProgressToCard(modelId);
  } catch (_e) {
    container.innerHTML = `<div style="color:#565f89;font-size:0.8rem;">${i18n.t('model.load_error')}</div>`;
  }
}

/**
 * Получить путь модели из карточки по типу транскрайбера. Если карточка
 * не отрендерилась или input пустой — НЕ возвращаем '', а откатываемся
 * в settingsState.originalConfig (тот, который только что прислал
 * backend). Иначе одно несвоевременное Save обнулит whisper_model_path
 * / model_path / qwen3asr_* и engine упадёт с ошибкой "из :".
 */
export function getModelPathFromCard(ttype: string): string {
  const input = document.querySelector<HTMLInputElement>(`.model-path-input[data-type="${ttype}"]`);
  const fromCard = input ? input.value.trim() : '';
  if (fromCard) return fromCard;
  const fieldByType: Record<string, keyof NonNullable<ReturnType<typeof settingsState.getOriginal>>> = {
    vosk: 'model_path',
    whisper: 'whisper_model_path',
    gigaam: 'gigaam_model_path',
    'gigaam-rnnt': 'gigaam_rnnt_model_path',
    qwen3asr: 'qwen3asr_model_path',
  };
  const field = fieldByType[ttype];
  const orig = settingsState.getOriginal();
  return (orig && field && (orig[field] as string)) || '';
}
