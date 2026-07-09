// pages/history/render.ts
//
// Rendering helpers для истории: одна запись = карточка с метой, dropdown'ом
// «выбор модели», текстом транскрибации (или кнопкой «Распознать»),
// действиями (play/copy/delete). Web Audio API для воспроизведения.

import { invoke } from '../../shared/lib/tauri';
import { i18n } from '../../shared/lib/i18n';
import { initCustomSelects } from '../../shared/ui/custom-select';

interface HistoryEntry {
  recording: { id: number; timestamp: number; duration_secs: number };
  transcriptions: Array<{ model_name: string; text: string; transcriber_type: string }>;
  audio_exists: boolean;
}

interface RenderOptions {
  entries: HistoryEntry[];
  total: number;
  historyList: HTMLElement;
  historyEmpty: HTMLElement;
  histPagination: HTMLElement;
  histPrev: HTMLButtonElement;
  histNext: HTMLButtonElement;
  histPageInfo: HTMLElement;
  perPage: number;
  currentPage: number;
  onReload: () => Promise<void>;
}

function fmtDate(ts: number): string {
  const d = new Date(ts * 1000);
  const locale = i18n.getLanguage() === 'en' ? 'en-US' : 'ru-RU';
  return d.toLocaleString(locale, {
    day: '2-digit', month: '2-digit', year: 'numeric',
    hour: '2-digit', minute: '2-digit',
  });
}

function fmtDuration(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  if (i18n.getLanguage() === 'en') {
    return m > 0 ? `${m}m ${s}s` : `${s}s`;
  }
  return m > 0 ? `${m}м ${s}с` : `${s}с`;
}

// Доступные типы моделей для кнопки "Распознать"
const MODEL_TYPES = [
  { type: 'vosk', label: 'Vosk' },
  { type: 'whisper', label: 'Whisper' },
  { type: 'gigaam-rnnt', label: 'GigaAM RNN-T' },
  { type: 'gigaam', label: 'GigaAM CTC' },
  { type: 'qwen3asr', label: 'Qwen3-ASR' },
];

// Одна опция dropdown'а «выбор модели» для записи
interface ModelOption { label: string; text: string | null; type: string; hasText: boolean; isOriginal?: boolean }

// Собираем модели: существующие транскрибации + по одной "нераспознано" для
// каждого типа, которого ещё нет. Оригинал — самая ранняя (последняя в DESC-массиве).
function buildModelOptions(trans: HistoryEntry['transcriptions']): ModelOption[] {
  const firstTrans = trans.length > 0 ? trans[trans.length - 1] : null;
  const originalModel = firstTrans ? firstTrans.model_name : null;
  const modelOptions: ModelOption[] = [];
  const seenTypes = new Set<string>();
  for (const t of trans) {
    modelOptions.push({
      label: t.model_name,
      text: t.text,
      type: t.transcriber_type,
      hasText: true,
      isOriginal: t.model_name === originalModel,
    });
    seenTypes.add(t.transcriber_type);
  }
  for (const m of MODEL_TYPES) {
    if (!seenTypes.has(m.type)) {
      modelOptions.push({
        label: m.label + ' ' + i18n.t('history.not_recognized'),
        text: null,
        type: m.type,
        hasText: false,
      });
    }
  }
  return modelOptions;
}

// Обработчик кнопки "Распознать" — запускает retranscribe, при ошибке откатывает подпись.
async function doRetranscribe(e: Event, onReload: () => Promise<void>): Promise<void> {
  const btn = e.target as HTMLButtonElement;
  const rid = parseInt(btn.dataset.rid || '0');
  const ttype = btn.dataset.type || '';
  btn.textContent = i18n.t('history.recognizing');
  btn.dataset.i18n = 'history.recognizing';
  btn.disabled = true;
  try {
    await invoke('retranscribe', { recordingId: rid, transcriberType: ttype });
    await onReload();
  } catch (err) {
    alert(i18n.t('toast.error') + ': ' + err);
    btn.textContent = i18n.t('history.recognize_btn');
    btn.dataset.i18n = 'history.recognize_btn';
    btn.disabled = false;
  }
}

// Заполняет contentDiv для выбранной модели: текст / кнопка «Распознать» / «аудио удалено».
// Единая точка — используется и на первом рендере, и при переключении модели в dropdown'е.
function fillContent(
  contentDiv: HTMLElement,
  opt: ModelOption | undefined,
  entry: HistoryEntry,
  onRetranscribe: (e: Event) => void,
): void {
  const r = entry.recording;
  contentDiv.innerHTML = '';
  contentDiv.className = '';
  if (opt && opt.hasText) {
    contentDiv.className = 'hist-text';
    contentDiv.textContent = opt.text;
  } else if (entry.audio_exists) {
    contentDiv.innerHTML = `<button class="hist-retranscribe-btn" data-rid="${r.id}" data-type="${opt ? opt.type : 'whisper'}" data-i18n="history.recognize_btn">${i18n.t('history.recognize_btn')}</button>`;
    contentDiv.querySelector('.hist-retranscribe-btn')?.addEventListener('click', onRetranscribe);
  } else {
    contentDiv.innerHTML = `<span style="color:#565f89;font-size:0.8rem;font-style:italic;" data-i18n="history.audio_deleted">${i18n.t('history.audio_deleted')}</span>`;
  }
}

// Первая строка карточки: дата, длительность + кнопки действий (play/copy/delete).
function buildMetaRow(entry: HistoryEntry): { metaDiv: HTMLElement; actionsDiv: HTMLElement } {
  const r = entry.recording;
  const metaDiv = document.createElement('div');
  metaDiv.className = 'hist-meta';
  metaDiv.innerHTML = `
      <span>${fmtDate(r.timestamp)}</span>
      <span>${fmtDuration(r.duration_secs)}</span>
    `;
  const actionsDiv = document.createElement('div');
  actionsDiv.className = 'hist-actions';
  actionsDiv.innerHTML = `
      ${entry.audio_exists ? `<button class="hist-btn play" data-i18n-title="history.play" title="${i18n.t('history.play')}" data-rid="${r.id}">
        <svg viewBox="0 0 24 24"><polygon points="5,3 19,12 5,21"/></svg>
      </button>` : ''}
      <button class="hist-btn copy" data-i18n-title="history.copy" title="${i18n.t('history.copy')}">
        <svg viewBox="0 0 24 24"><path d="M16 1H4c-1.1 0-2 .9-2 2v14h2V3h12V1zm3 4H8c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h11c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm0 16H8V7h11v14z"/></svg>
      </button>
      <button class="hist-btn delete" data-i18n-title="history.delete" title="${i18n.t('history.delete')}">
        <svg viewBox="0 0 24 24"><path d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z"/></svg>
      </button>
    `;
  metaDiv.appendChild(actionsDiv);
  return { metaDiv, actionsDiv };
}

// Вторая строка карточки: кастомный dropdown с моделями.
function buildModelSelect(
  modelOptions: ModelOption[],
  defaultModel: string,
  originalModel: string | null,
): HTMLElement {
  const selectWrap = document.createElement('div');
  selectWrap.className = 'custom-select';
  selectWrap.dataset.value = defaultModel;
  selectWrap.style.margin = '0.4rem 0';
  selectWrap.innerHTML = `
      <div class="custom-select-trigger${defaultModel === originalModel ? ' original' : ''}">${defaultModel}${defaultModel === originalModel ? ' ★' : ''}</div>
      <div class="custom-select-options">
        ${modelOptions.map(m =>
          `<div class="custom-select-option${m.label === defaultModel ? ' selected' : ''}" data-value="${m.label}" data-type="${m.type}" data-has-text="${m.hasText}">${m.label}${m.isOriginal ? ' ★' : ''}</div>`,
        ).join('')}
      </div>
    `;
  return selectWrap;
}

// Сборка одной карточки записи + привязка всех обработчиков (dropdown/copy/delete/play).
function renderEntryCard(entry: HistoryEntry, onReload: () => Promise<void>): HTMLElement {
  const r = entry.recording;
  const trans = entry.transcriptions;
  // Самая ранняя транскрибация — последняя в массиве (DESC порядок)
  const firstTrans = trans.length > 0 ? trans[trans.length - 1] : null;
  const defaultModel = firstTrans ? firstTrans.model_name : MODEL_TYPES[0].label;
  const originalModel = firstTrans ? firstTrans.model_name : null;
  const modelOptions = buildModelOptions(trans);

  const div = document.createElement('div');
  div.className = 'hist-entry';

  const { metaDiv, actionsDiv } = buildMetaRow(entry);
  div.appendChild(metaDiv);

  const selectWrap = buildModelSelect(modelOptions, defaultModel, originalModel);
  div.appendChild(selectWrap);

  // Третья строка: текст или кнопка "Распознать"
  const contentDiv = document.createElement('div');
  const onRetranscribe = (e: Event): void => { void doRetranscribe(e, onReload); };
  fillContent(contentDiv, modelOptions.find(m => m.label === defaultModel), entry, onRetranscribe);
  div.appendChild(contentDiv);

  // Переключение модели
  selectWrap.addEventListener('change', () => {
    const selectedLabel = selectWrap.dataset.value || '';
    const opt = modelOptions.find(m => m.label === selectedLabel);
    const trigger = selectWrap.querySelector('.custom-select-trigger') as HTMLElement;
    trigger.classList.toggle('original', !!(opt && opt.isOriginal));
    trigger.textContent = selectedLabel + (opt && opt.isOriginal ? ' ★' : '');
    fillContent(contentDiv, opt, entry, onRetranscribe);
  });

  // Копировать
  actionsDiv.querySelector('.hist-btn.copy')?.addEventListener('click', async () => {
    const textEl = div.querySelector('.hist-text');
    if (textEl) await navigator.clipboard.writeText(textEl.textContent || '');
  });

  // Удалить
  actionsDiv.querySelector('.hist-btn.delete')?.addEventListener('click', async () => {
    await invoke('delete_history_entry', { recordingId: r.id });
    await onReload();
  });

  // Воспроизвести запись
  const playBtn = actionsDiv.querySelector<HTMLButtonElement>('.hist-btn.play');
  if (playBtn) {
    playBtn.addEventListener('click', () => playAudio(parseInt(playBtn.dataset.rid || '0'), playBtn));
  }

  return div;
}

// Пагинация: показать/скрыть блок + обновить состояние кнопок и подпись.
function renderPagination(opts: RenderOptions): void {
  const { total, perPage, currentPage, histPagination, histPrev, histNext, histPageInfo } = opts;
  const totalPages = Math.ceil(total / perPage);
  if (totalPages > 1) {
    histPagination.classList.add('visible');
    histPrev.disabled = currentPage === 0;
    histNext.disabled = currentPage >= totalPages - 1;
    histPageInfo.textContent = i18n.t('history.page', { current: currentPage + 1, total: totalPages });
  } else {
    histPagination.classList.remove('visible');
  }
}

export function renderHistoryEntries(opts: RenderOptions): void {
  const { entries, historyList, historyEmpty, histPagination, onReload } = opts;
  historyList.innerHTML = '';
  if (entries.length === 0) {
    historyEmpty.style.display = 'block';
    histPagination.classList.remove('visible');
    return;
  }
  historyEmpty.style.display = 'none';

  for (const entry of entries) {
    historyList.appendChild(renderEntryCard(entry, onReload));
  }

  // Инициализация всех кастомных select'ов — один проход после добавления всех
  // записей. Раньше initCustomSelects() звался внутри цикла на каждую запись:
  // функция сканирует ВСЕ .custom-select на странице → O(N²) при N записях.
  // Идемпотентность делала это безопасным, но не быстрым. Один вызов → O(N).
  initCustomSelects();

  renderPagination(opts);
}

// === Воспроизведение аудио через Web Audio API ===
const SVG_PLAY = '<svg viewBox="0 0 24 24"><polygon points="5,3 19,12 5,21"/></svg>';
const SVG_STOP = '<svg viewBox="0 0 24 24"><rect x="4" y="4" width="16" height="16" rx="2"/></svg>';
const SVG_PAUSE = '<svg viewBox="0 0 24 24"><rect x="5" y="4" width="4" height="16"/><rect x="15" y="4" width="4" height="16"/></svg>';
const SVG_RESUME = '<svg viewBox="0 0 24 24"><polygon points="5,3 19,12 5,21"/></svg>';

interface PlayerState {
  source: AudioBufferSourceNode;
  ctx: AudioContext;
  playBtn: HTMLButtonElement;
  pauseBtn: HTMLButtonElement | null;
  paused: boolean;
}

let currentPlayer: PlayerState | null = null;

export function stopCurrentPlayer(): void {
  if (!currentPlayer) return;
  try { currentPlayer.source.stop(); } catch { /* already stopped */ }
  void currentPlayer.ctx.close();
  currentPlayer.playBtn.innerHTML = SVG_PLAY;
  currentPlayer.playBtn.classList.remove('playing');
  if (currentPlayer.pauseBtn) currentPlayer.pauseBtn.remove();
  currentPlayer = null;
}

async function playAudio(recordingId: number, playBtn: HTMLButtonElement): Promise<void> {
  // Если уже играет эту запись — стоп
  if (currentPlayer && currentPlayer.playBtn === playBtn) {
    stopCurrentPlayer();
    return;
  }
  // Остановить предыдущее
  stopCurrentPlayer();

  playBtn.innerHTML = SVG_STOP;
  playBtn.classList.add('playing');

  try {
    const result = await invoke<{ data: string; sample_rate: number }>('get_audio_data', { recordingId });
    const raw = atob(result.data);
    const bytes = new Uint8Array(raw.length);
    for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);

    const view = new DataView(bytes.buffer);
    const numSamples = bytes.length / 2;
    const audioCtx = new AudioContext({ sampleRate: result.sample_rate });
    const buffer = audioCtx.createBuffer(1, numSamples, result.sample_rate);
    const channel = buffer.getChannelData(0);
    for (let i = 0; i < numSamples; i++) {
      channel[i] = view.getInt16(i * 2, true) / 32768.0;
    }

    const source = audioCtx.createBufferSource();
    source.buffer = buffer;
    source.connect(audioCtx.destination);

    // Кнопка паузы — вставляем перед play/stop
    const pauseBtn = document.createElement('button');
    pauseBtn.className = 'hist-btn play';
    pauseBtn.title = i18n.t('mic.pause');
    pauseBtn.dataset.i18nTitle = 'mic.pause';
    pauseBtn.innerHTML = SVG_PAUSE;
    playBtn.parentNode?.insertBefore(pauseBtn, playBtn);

    let paused = false;
    pauseBtn.addEventListener('click', () => {
      if (!currentPlayer) return;
      if (paused) {
        void audioCtx.resume();
        pauseBtn.innerHTML = SVG_PAUSE;
        pauseBtn.title = i18n.t('mic.pause');
        pauseBtn.dataset.i18nTitle = 'mic.pause';
        paused = false;
      } else {
        void audioCtx.suspend();
        pauseBtn.innerHTML = SVG_RESUME;
        pauseBtn.title = i18n.t('mic.resume');
        pauseBtn.dataset.i18nTitle = 'mic.resume';
        paused = true;
      }
    });

    currentPlayer = { source, ctx: audioCtx, playBtn, pauseBtn, paused };

    source.onended = (): void => {
      if (currentPlayer && currentPlayer.source === source) {
        stopCurrentPlayer();
      }
    };
    source.start();
  } catch (e) {
    playBtn.innerHTML = SVG_PLAY;
    playBtn.classList.remove('playing');
    console.error('Ошибка воспроизведения:', e);
  }
}
