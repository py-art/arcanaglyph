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

export function mountMainControls(): { onModelReady: () => void } {
  const statusEl = document.getElementById('status')!;
  const resultWrap = document.getElementById('result-wrap')!;
  const resultEl = document.getElementById('result')!;
  const copyBtn = document.getElementById('copy-btn')!;
  const micBtn = document.getElementById('mic-btn') as HTMLElement;
  const micGlow = document.getElementById('mic-glow')!;
  const timerEl = document.getElementById('timer')!;
  const levelBar = document.getElementById('level-bar')!;
  const levelFill = document.getElementById('level-fill') as HTMLElement;
  const controlsEl = document.getElementById('controls')!;
  const stopBtn = document.getElementById('stop-btn')!;
  const pauseBtn = document.getElementById('pause-btn')!;

  const s: State = {
    recording: false,
    transcribing: false,
    modelReady: false,
    timerMs: 0,
    timerId: null,
    levelId: null,
  };

  micBtn.style.opacity = '0.4';
  micBtn.style.cursor = 'default';

  // === Buttons ===
  micBtn.addEventListener('click', () => {
    if (!s.recording && s.modelReady) void invoke('trigger');
  });
  // Замечание: во время транскрибации кнопка скрыта (см. transcribing event).
  stopBtn.addEventListener('click', () => {
    if (s.recording && !s.transcribing) void invoke('trigger');
  });
  pauseBtn.addEventListener('click', () => {
    if (s.recording) void invoke('pause');
  });
  copyBtn.addEventListener('click', async () => {
    await navigator.clipboard.writeText(resultEl.textContent ?? '');
    copyBtn.classList.add('copied');
    setTimeout(() => copyBtn.classList.remove('copied'), 1500);
  });

  const onModelReady = (): void => {
    if (s.modelReady) return;
    s.modelReady = true;
    statusEl.textContent = t('status.ready');
    statusEl.dataset.i18n = 'status.ready';
    statusEl.className = '';
    micBtn.style.opacity = '';
    micBtn.style.cursor = 'pointer';
    void updateModelBadge();
  };

  // === Engine events ===
  void listen('engine://model-loaded', onModelReady);

  void listen<{ model?: string }>('engine://model-loading', ev => {
    const modelName = ev?.payload?.model || '';
    s.modelReady = false;
    statusEl.textContent = t('status.loading_model').replace('{model}', modelName);
    statusEl.dataset.i18n = 'status.loading_model';
    statusEl.className = '';
    micBtn.style.opacity = '0.4';
    micBtn.style.cursor = 'default';
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
    statusEl.textContent = t('status.downloading_model').replace('{percent}', String(pct));
  });
  void listen('download://complete', () => {
    if (s.modelReady) return;
    statusEl.textContent = t('status.loading');
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
      } catch (_) {}
      await new Promise(r => setTimeout(r, 200));
    }
  })();

  void listen('engine://recording-started', () => {
    s.recording = true;
    resultWrap.classList.remove('visible');
    resultEl.textContent = '';
    resultEl.classList.remove('error');

    micBtn.classList.add('recording');
    micGlow.classList.add('recording');
    statusEl.textContent = t('status.recording');
    statusEl.dataset.i18n = 'status.recording';
    statusEl.className = 'recording';

    controlsEl.classList.add('active');
    levelBar.classList.add('active');
    timerEl.classList.add('active');

    s.timerMs = 0;
    timerEl.textContent = '00:00.00';
    s.timerId = setInterval(() => {
      s.timerMs += 70;
      timerEl.textContent = fmt(s.timerMs);
    }, 70);
    s.levelId = setInterval(async () => {
      const level = await invoke<number>('get_audio_level');
      levelFill.style.width = level + '%';
    }, 100);
  });

  void listen('engine://recording-paused', () => {
    statusEl.textContent = t('status.paused');
    statusEl.dataset.i18n = 'status.paused';
    statusEl.className = 'recording';
    if (s.timerId) clearInterval(s.timerId);
    if (s.levelId) clearInterval(s.levelId);
    levelFill.style.width = '0%';
    pauseBtn.innerHTML = '<svg viewBox="0 0 24 24"><polygon points="6,4 20,12 6,20"/></svg>';
    pauseBtn.classList.add('resume');
    pauseBtn.title = t('mic.resume');
    pauseBtn.dataset.i18nTitle = 'mic.resume';
  });

  void listen('engine://recording-resumed', () => {
    statusEl.textContent = t('status.recording');
    statusEl.dataset.i18n = 'status.recording';
    statusEl.className = 'recording';
    s.timerId = setInterval(() => {
      s.timerMs += 70;
      timerEl.textContent = fmt(s.timerMs);
    }, 70);
    s.levelId = setInterval(async () => {
      const level = await invoke<number>('get_audio_level');
      levelFill.style.width = level + '%';
    }, 100);
    levelBar.classList.add('active');
    pauseBtn.innerHTML = '<svg viewBox="0 0 24 24"><rect x="6" y="4" width="4" height="16"/><rect x="14" y="4" width="4" height="16"/></svg>';
    pauseBtn.classList.remove('resume');
    pauseBtn.title = t('mic.pause');
    pauseBtn.dataset.i18nTitle = 'mic.pause';
  });

  void listen('engine://transcribing', async () => {
    s.transcribing = true;
    statusEl.textContent = t('status.transcribing');
    statusEl.dataset.i18n = 'status.transcribing';
    statusEl.className = 'recording';
    if (s.timerId) clearInterval(s.timerId);
    if (s.levelId) clearInterval(s.levelId);
    levelFill.style.width = '0%';
    levelBar.classList.remove('active');
    controlsEl.classList.remove('active');
    // Whisper поддерживает cancel — показываем кнопку Стоп.
    try {
      const canCancel = await invoke<boolean>('active_supports_cancel');
      if (canCancel) controlsEl.classList.add('transcribing');
    } catch (_) {}
  });

  void listen<{ text: string }>('engine://transcription-result', ev => {
    resultEl.textContent = ev.payload.text || t('result.empty');
    resultWrap.classList.add('visible');
    requestAnimationFrame(() => {
      resultEl.classList.toggle('scrollable', resultEl.scrollHeight > resultEl.clientHeight);
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
    resultEl.textContent = hint ? `${msg} — ${hint}` : msg;
    resultEl.classList.add('error');
    resultWrap.classList.add('visible');
  });

  void listen('engine://finished-processing', () => {
    s.recording = false;
    s.transcribing = false;
    void updateModelBadge();

    micBtn.classList.remove('recording');
    micGlow.classList.remove('recording');
    statusEl.textContent = t('status.ready');
    statusEl.dataset.i18n = 'status.ready';
    statusEl.className = '';

    controlsEl.classList.remove('active');
    controlsEl.classList.remove('transcribing');
    levelBar.classList.remove('active');
    timerEl.classList.remove('active');

    if (s.timerId) clearInterval(s.timerId);
    if (s.levelId) clearInterval(s.levelId);
    levelFill.style.width = '0%';
  });

  return { onModelReady };
}
