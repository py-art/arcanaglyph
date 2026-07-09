// widgets/main-controls/main-controls.ts
//
// Главный recording widget: микрофон-кнопка, статус, результат
// транскрибации, таймер, level-meter, controls (stop/pause).
// Слушает события engine: recording-started/paused/resumed/transcribing/
// transcription-result/error/finished-processing/model-loading/loaded/
// fallback и реагирует на них (UI state machine).
//
// Этот widget один и тот же для главной страницы — он не разбит на
// отдельные mic-button / level-meter / status-display модули намеренно:
// они тесно переплетены состоянием (recording, transcribing, modelReady,
// timerId, levelId), и любой реальный split привёл бы к экспорту mutable
// state наружу. Лучше монолит с единым lifecycle.
//
// Внутри файла обработчики сгруппированы по концернам в приватные wire-*
// функции (кнопки / model-lifecycle / recording / transcription) и три
// фабрики helper'ов (startLevelTimers / resetToIdle / onModelReady). State
// и DOM-refs остаются приватными для модуля — наружу утекает только
// onModelReady, как и раньше.

import { invoke, listen, isCancelled, errorHint } from '../../shared/lib/tauri';
import type { ApiError } from '../../shared/lib/tauri';
import { t } from '../../shared/lib/i18n';
import { showToast } from '../../shared/ui/toast';
import { updateModelBadge } from '../model-badge/model-badge';

function fmt(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  const cs = Math.floor((ms % 1000) / 10);
  return `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}.${String(cs).padStart(2, '0')}`;
}

interface State {
  recording: boolean;
  transcribing: boolean;
  modelReady: boolean;
  timerMs: number;
  timerId: ReturnType<typeof setInterval> | null;
  levelId: ReturnType<typeof setInterval> | null;
}

// DOM-элементы виджета, собранные один раз при монтировании.
interface Refs {
  statusEl: HTMLElement;
  resultWrap: HTMLElement;
  resultEl: HTMLElement;
  copyBtn: HTMLElement;
  micBtn: HTMLElement;
  micGlow: HTMLElement;
  timerEl: HTMLElement;
  levelBar: HTMLElement;
  levelFill: HTMLElement;
  controlsEl: HTMLElement;
  stopBtn: HTMLElement;
  pauseBtn: HTMLElement;
}

function queryRefs(): Refs {
  return {
    statusEl: document.getElementById('status')!,
    resultWrap: document.getElementById('result-wrap')!,
    resultEl: document.getElementById('result')!,
    copyBtn: document.getElementById('copy-btn')!,
    micBtn: document.getElementById('mic-btn') as HTMLElement,
    micGlow: document.getElementById('mic-glow')!,
    timerEl: document.getElementById('timer')!,
    levelBar: document.getElementById('level-bar')!,
    levelFill: document.getElementById('level-fill') as HTMLElement,
    controlsEl: document.getElementById('controls')!,
    stopBtn: document.getElementById('stop-btn')!,
    pauseBtn: document.getElementById('pause-btn')!,
  };
}

// Запуск таймера записи + level-meter с предварительной очисткой. И
// recording-started, и recording-resumed раньше создавали setInterval БЕЗ
// clearInterval — при потерянном/сдвоенном событии интервалы накладывались
// и таймер «дёргался». Очищаем перед запуском → ровно один активный интервал
// каждого вида. timerMs здесь НЕ сбрасываем (resume продолжает счёт).
function createStartLevelTimers(refs: Refs, s: State): () => void {
  return () => {
    if (s.timerId) clearInterval(s.timerId);
    if (s.levelId) clearInterval(s.levelId);
    s.timerId = setInterval(() => {
      s.timerMs += 70;
      refs.timerEl.textContent = fmt(s.timerMs);
    }, 70);
    s.levelId = setInterval(async () => {
      const level = await invoke<number>('get_audio_level');
      refs.levelFill.style.width = level + '%';
    }, 100);
  };
}

// Возврат UI в idle. Вынесено из engine://finished-processing, чтобы тот же
// сброс мог вызвать watchdog при потере broadcast-события (см. ниже).
function createResetToIdle(refs: Refs, s: State): () => void {
  return () => {
    s.recording = false;
    s.transcribing = false;
    void updateModelBadge();

    refs.micBtn.classList.remove('recording');
    refs.micGlow.classList.remove('recording');
    refs.statusEl.textContent = t('status.ready');
    refs.statusEl.dataset.i18n = 'status.ready';
    refs.statusEl.className = '';

    refs.controlsEl.classList.remove('active');
    refs.controlsEl.classList.remove('transcribing');
    refs.levelBar.classList.remove('active');
    refs.timerEl.classList.remove('active');

    if (s.timerId) clearInterval(s.timerId);
    if (s.levelId) clearInterval(s.levelId);
    refs.levelFill.style.width = '0%';
  };
}

function createOnModelReady(refs: Refs, s: State): () => void {
  return () => {
    if (s.modelReady) return;
    s.modelReady = true;
    refs.statusEl.textContent = t('status.ready');
    refs.statusEl.dataset.i18n = 'status.ready';
    refs.statusEl.className = '';
    refs.micBtn.style.opacity = '';
    refs.micBtn.style.cursor = 'pointer';
    void updateModelBadge();
  };
}

// === Buttons ===
function wireButtons(refs: Refs, s: State): void {
  refs.micBtn.addEventListener('click', () => {
    if (!s.recording && s.modelReady) void invoke('trigger');
  });
  // Замечание: во время транскрибации кнопка скрыта (см. transcribing event).
  refs.stopBtn.addEventListener('click', () => {
    if (s.recording && !s.transcribing) void invoke('trigger');
  });
  refs.pauseBtn.addEventListener('click', () => {
    if (s.recording) void invoke('pause');
  });
  refs.copyBtn.addEventListener('click', async () => {
    await navigator.clipboard.writeText(refs.resultEl.textContent ?? '');
    refs.copyBtn.classList.add('copied');
    setTimeout(() => refs.copyBtn.classList.remove('copied'), 1500);
  });
}

// === Model lifecycle events ===
function wireModelLifecycle(refs: Refs, s: State, onModelReady: () => void): void {
  void listen('engine://model-loaded', onModelReady);

  void listen<{ model?: string }>('engine://model-loading', ev => {
    const modelName = ev?.payload?.model || '';
    s.modelReady = false;
    refs.statusEl.textContent = t('status.loading_model').replace('{model}', modelName);
    refs.statusEl.dataset.i18n = 'status.loading_model';
    refs.statusEl.className = '';
    refs.micBtn.style.opacity = '0.4';
    refs.micBtn.style.cursor = 'default';
  });

  void listen<{ original?: string; fallback?: string }>('engine://fallback', ev => {
    const original = ev?.payload?.original ?? '';
    const fallback = ev?.payload?.fallback ?? '';
    const msg = t('toast.engine_fallback')
      .replace('{original}', original)
      .replace('{fallback}', fallback);
    showToast(msg, 'error', 5000);
  });

  void listen<{ percent?: number }>('download://progress', ev => {
    if (s.modelReady) return;
    const pct = ev?.payload?.percent ?? 0;
    refs.statusEl.textContent = t('status.downloading_model').replace('{percent}', String(pct));
  });
  void listen('download://complete', () => {
    if (s.modelReady) return;
    refs.statusEl.textContent = t('status.loading');
  });

  // Поллинг на случай если model-loaded event пришёл до подписки.
  void (async () => {
    while (!s.modelReady) {
      try {
        const loaded = await invoke<boolean>('is_model_loaded');
        if (loaded) {
          onModelReady();
          break;
        }
      } catch { /* временный сбой invoke — poll повторит через 200ms */ }
      await new Promise(r => setTimeout(r, 200));
    }
  })();
}

// === Recording events ===
function wireRecordingEvents(refs: Refs, s: State, startLevelTimers: () => void): void {
  void listen('engine://recording-started', () => {
    s.recording = true;
    refs.resultWrap.classList.remove('visible');
    refs.resultEl.textContent = '';
    refs.resultEl.classList.remove('error');

    refs.micBtn.classList.add('recording');
    refs.micGlow.classList.add('recording');
    refs.statusEl.textContent = t('status.recording');
    refs.statusEl.dataset.i18n = 'status.recording';
    refs.statusEl.className = 'recording';

    refs.controlsEl.classList.add('active');
    refs.levelBar.classList.add('active');
    refs.timerEl.classList.add('active');

    s.timerMs = 0;
    refs.timerEl.textContent = '00:00.00';
    startLevelTimers();
  });

  void listen('engine://recording-paused', () => {
    refs.statusEl.textContent = t('status.paused');
    refs.statusEl.dataset.i18n = 'status.paused';
    refs.statusEl.className = 'recording';
    if (s.timerId) clearInterval(s.timerId);
    if (s.levelId) clearInterval(s.levelId);
    refs.levelFill.style.width = '0%';
    refs.pauseBtn.innerHTML = '<svg viewBox="0 0 24 24"><polygon points="6,4 20,12 6,20"/></svg>';
    refs.pauseBtn.classList.add('resume');
    refs.pauseBtn.title = t('mic.resume');
    refs.pauseBtn.dataset.i18nTitle = 'mic.resume';
  });

  void listen('engine://recording-resumed', () => {
    refs.statusEl.textContent = t('status.recording');
    refs.statusEl.dataset.i18n = 'status.recording';
    refs.statusEl.className = 'recording';
    startLevelTimers();
    refs.levelBar.classList.add('active');
    refs.pauseBtn.innerHTML = '<svg viewBox="0 0 24 24"><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></svg>';
    refs.pauseBtn.classList.remove('resume');
    refs.pauseBtn.title = t('mic.pause');
    refs.pauseBtn.dataset.i18nTitle = 'mic.pause';
  });
}

// === Transcription events ===
function wireTranscriptionEvents(refs: Refs, s: State, resetToIdle: () => void): void {
  void listen('engine://transcribing', async () => {
    s.transcribing = true;
    refs.statusEl.textContent = t('status.transcribing');
    refs.statusEl.dataset.i18n = 'status.transcribing';
    refs.statusEl.className = 'recording';
    if (s.timerId) clearInterval(s.timerId);
    if (s.levelId) clearInterval(s.levelId);
    refs.levelFill.style.width = '0%';
    refs.levelBar.classList.remove('active');
    refs.controlsEl.classList.remove('active');
    // Whisper поддерживает cancel — показываем кнопку Стоп.
    try {
      const canCancel = await invoke<boolean>('active_supports_cancel');
      if (canCancel) refs.controlsEl.classList.add('transcribing');
    } catch { /* best-effort: не узнали cancel-capability — кнопку «Стоп» просто не показываем */ }
  });

  void listen<{ text: string }>('engine://transcription-result', ev => {
    refs.resultEl.textContent = ev.payload.text || t('result.empty');
    refs.resultWrap.classList.add('visible');
    requestAnimationFrame(() => {
      refs.resultEl.classList.toggle(
        'scrollable',
        refs.resultEl.scrollHeight > refs.resultEl.clientHeight,
      );
    });
  });

  void listen<ApiError>('engine://error', ev => {
    // Cancelled — не ошибка, пользователь сам нажал «Стоп»: не подсвечиваем
    // result-блок красным и не показываем сообщение. UI вернётся в idle через
    // engine://finished-processing.
    if (isCancelled(ev.payload)) return;
    const msg = ev.payload.message || t('result.unknown_error');
    const hint = errorHint(ev.payload);
    // Если есть hint — показываем «<сообщение> — <подсказка>». Два строки в
    // одном result-блоке: основное сообщение + что делать.
    refs.resultEl.textContent = hint ? `${msg} — ${hint}` : msg;
    refs.resultEl.classList.add('error');
    refs.resultWrap.classList.add('visible');
  });

  void listen('engine://finished-processing', resetToIdle);
}

// Watchdog рассинхрона: если broadcast-событие engine://finished-processing
// потерялось (лаг/переполнение канала), UI мог залипнуть в «recording». Раз в
// ~1.5с сверяемся с реальным состоянием движка: если он уже не пишет и мы не
// в фазе транскрибации, а UI всё ещё думает что recording — возвращаем idle.
function startFinishWatchdog(s: State, resetToIdle: () => void): void {
  setInterval(async () => {
    if (!s.recording || s.transcribing) return;
    try {
      const stillRecording = await invoke<boolean>('is_recording');
      if (!stillRecording) resetToIdle();
    } catch { /* тик watchdog: ошибка одного опроса не критична, повторим через 1.5с */ }
  }, 1500);
}

export function mountMainControls(): { onModelReady: () => void } {
  const refs = queryRefs();

  const s: State = {
    recording: false,
    transcribing: false,
    modelReady: false,
    timerMs: 0,
    timerId: null,
    levelId: null,
  };

  refs.micBtn.style.opacity = '0.4';
  refs.micBtn.style.cursor = 'default';

  const startLevelTimers = createStartLevelTimers(refs, s);
  const resetToIdle = createResetToIdle(refs, s);
  const onModelReady = createOnModelReady(refs, s);

  wireButtons(refs, s);
  wireModelLifecycle(refs, s, onModelReady);
  wireRecordingEvents(refs, s, startLevelTimers);
  wireTranscriptionEvents(refs, s, resetToIdle);
  startFinishWatchdog(s, resetToIdle);

  return { onModelReady };
}
