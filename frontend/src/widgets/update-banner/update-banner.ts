// widgets/update-banner/update-banner.ts
//
// Bottom-fixed banner с двумя UI-режимами:
//
// • available — есть новый релиз. Кнопки «Обновить» / «Что нового» / «×».
// • applying  — пользователь нажал «Обновить»: indeterminate progress
//   + «Перезапустить» + «×». Persistent через UpdateState.applying_version,
//   восстанавливается между запусками приложения до тех пор пока
//   APP_VERSION не догонит applying_version.
//
// События backend'а: `update://available` (есть новый релиз) и
// `update://applying` (восстановление режима при старте). На mount
// дополнительно опрашиваем `get_update_applying` чтобы не зависеть от
// порядка emit'ов.
//
// Внутри файла логика разложена по концернам: queryRefs (сбор + guard),
// render (state-машина), install-poll helpers, и три wire-функции
// (available-actions / applying-actions / banner-sources). Общий mutable
// state держится в `Ctx`, чтобы разные обработчики могли и читать, и
// переприсваивать его по ссылке.

import { invoke, listen } from '../../shared/lib/tauri';
import { t } from '../../shared/lib/i18n';
import { showToast } from '../../shared/ui/toast';

interface UpdateInfo {
  latest_version: string;
  release_url: string;
  published_at: string;
}

type BannerState =
  | { kind: 'idle' }
  | { kind: 'available'; info: UpdateInfo }
  | { kind: 'applying'; version: string };

// DOM-элементы баннера. applyBtn/restartBtn — кнопки (нужен .disabled).
interface Refs {
  banner: HTMLElement;
  textAvailable: HTMLElement;
  textApplying: HTMLElement;
  versionEl: HTMLElement;
  applyingVersionEl: HTMLElement;
  progress: HTMLElement;
  actionsAvailable: HTMLElement;
  actionsApplying: HTMLElement;
  applyBtn: HTMLButtonElement;
  notesBtn: HTMLElement;
  dismissBtn: HTMLElement;
  restartBtn: HTMLButtonElement;
  dismissApplyingBtn: HTMLElement;
}

// Общий mutable-контекст виджета: state переприсваивается целиком (union),
// installPollTimer живёт между рендерами — оба нужны нескольким обработчикам.
interface Ctx {
  refs: Refs;
  state: BannerState;
  installPollTimer: ReturnType<typeof setInterval> | null;
}

function queryRefs(): Refs | null {
  const banner = document.getElementById('update-banner');
  const textAvailable = document.getElementById('update-banner-text-available');
  const textApplying = document.getElementById('update-banner-text-applying');
  const versionEl = document.getElementById('update-banner-version');
  const applyingVersionEl = document.getElementById('update-banner-applying-version');
  const progress = document.getElementById('update-banner-progress');
  const actionsAvailable = document.getElementById('update-banner-actions-available');
  const actionsApplying = document.getElementById('update-banner-actions-applying');
  const applyBtn = document.getElementById('update-banner-apply') as HTMLButtonElement | null;
  const notesBtn = document.getElementById('update-banner-notes');
  const dismissBtn = document.getElementById('update-banner-dismiss');
  const restartBtn = document.getElementById('update-banner-restart') as HTMLButtonElement | null;
  const dismissApplyingBtn = document.getElementById('update-banner-dismiss-applying');

  if (
    !banner || !textAvailable || !textApplying || !versionEl || !applyingVersionEl ||
    !progress || !actionsAvailable || !actionsApplying ||
    !applyBtn || !notesBtn || !dismissBtn || !restartBtn || !dismissApplyingBtn
  ) return null;

  return {
    banner, textAvailable, textApplying, versionEl, applyingVersionEl, progress,
    actionsAvailable, actionsApplying, applyBtn, notesBtn, dismissBtn, restartBtn,
    dismissApplyingBtn,
  };
}

function stopInstallPoll(ctx: Ctx): void {
  if (ctx.installPollTimer !== null) {
    clearInterval(ctx.installPollTimer);
    ctx.installPollTimer = null;
  }
}

// Баг #3: «Перезапустить» нельзя давать, пока install.sh ещё идёт (иначе
// перезапуск стартует СТАРУЮ версию). В applying-режиме кнопка disabled, а
// мы опрашиваем `update_install_ready` (флаг от терминала об exit 0) и
// разблокируем её только по факту завершения установки.
function startInstallPoll(ctx: Ctx): void {
  stopInstallPoll(ctx);
  const check = async (): Promise<void> => {
    let ready = false;
    try {
      ready = await invoke<boolean>('update_install_ready');
    } catch {
      return;
    }
    if (ready && ctx.state.kind === 'applying') {
      ctx.refs.restartBtn.disabled = false;
      ctx.refs.progress.hidden = true; // установка завершилась — прогресс больше не нужен
      stopInstallPoll(ctx);
    }
  };
  void check();
  ctx.installPollTimer = setInterval(() => void check(), 1500);
}

function render(ctx: Ctx, next: BannerState): void {
  ctx.state = next;
  const r = ctx.refs;
  switch (next.kind) {
    case 'idle':
      stopInstallPoll(ctx);
      r.banner.classList.remove('visible');
      return;
    case 'available':
      stopInstallPoll(ctx);
      r.versionEl.textContent = next.info.latest_version;
      r.textAvailable.hidden = false;
      r.textApplying.hidden = true;
      r.progress.hidden = true;
      r.actionsAvailable.hidden = false;
      r.actionsApplying.hidden = true;
      r.applyBtn.disabled = false;
      r.banner.classList.add('visible');
      return;
    case 'applying':
      r.applyingVersionEl.textContent = next.version;
      r.textAvailable.hidden = true;
      r.textApplying.hidden = false;
      r.progress.hidden = false;
      r.actionsAvailable.hidden = true;
      r.actionsApplying.hidden = false;
      // disabled до подтверждения установки; разблокирует startInstallPoll.
      r.restartBtn.disabled = true;
      r.banner.classList.add('visible');
      startInstallPoll(ctx);
      return;
  }
}

// Кнопки available-режима: «Обновить» / «Что нового» / «×».
function wireAvailableActions(ctx: Ctx): void {
  const r = ctx.refs;
  r.applyBtn.addEventListener('click', async () => {
    if (ctx.state.kind !== 'available') return;
    const info = ctx.state.info;
    // Переключаем UI до invoke — пользователь видит мгновенный фидбек,
    // даже если терминал ещё стартует.
    render(ctx, { kind: 'applying', version: info.latest_version });
    try {
      await invoke('apply_update', { latestVersion: info.latest_version });
      showToast(t('update.toast_terminal_started'), 'success', 5000);
    } catch (e) {
      // Откат: backend сам очистил applying_version при ошибке.
      render(ctx, { kind: 'available', info });
      showToast(t('update.toast_apply_failed', { err: String(e) }), 'error', 6000);
    }
  });

  r.notesBtn.addEventListener('click', async () => {
    if (ctx.state.kind !== 'available') return;
    try {
      await invoke('open_release_notes', { url: ctx.state.info.release_url });
    } catch (e) {
      showToast(String(e), 'error', 4000);
    }
  });

  r.dismissBtn.addEventListener('click', async () => {
    if (ctx.state.kind !== 'available') {
      render(ctx, { kind: 'idle' });
      return;
    }
    const version = ctx.state.info.latest_version;
    try {
      await invoke('dismiss_update', { version });
    } catch {
      // swallow — UI всё равно скрываем
    }
    render(ctx, { kind: 'idle' });
  });
}

// Кнопки applying-режима: «Перезапустить» / «×».
function wireApplyingActions(ctx: Ctx): void {
  const r = ctx.refs;
  r.restartBtn.addEventListener('click', async () => {
    if (ctx.state.kind !== 'applying') return;
    r.restartBtn.disabled = true;
    try {
      await invoke('restart_app');
      // Если процесс не вышел — оставляем кнопку disabled (Tauri exit
      // отрабатывает асинхронно, фронт ещё живёт ~миллисекунды).
    } catch (e) {
      r.restartBtn.disabled = false;
      showToast(t('update.toast_restart_failed', { err: String(e) }), 'error', 6000);
    }
  });

  r.dismissApplyingBtn.addEventListener('click', async () => {
    try {
      await invoke('clear_update_applying');
    } catch {
      // swallow
    }
    render(ctx, { kind: 'idle' });
  });
}

// Источники состояния: manual-API из About, backend-события и mount-опрос.
function wireBannerSources(ctx: Ctx): void {
  /** Метод-API для manual триггера баннера (используется setupAboutPage). */
  (window as Window & { __showUpdateBanner?: (info: UpdateInfo) => void }).__showUpdateBanner =
    (info: UpdateInfo): void => {
      // Manual-вызов из About не должен перетирать applying-режим.
      if (ctx.state.kind === 'applying') return;
      render(ctx, { kind: 'available', info });
    };

  void listen<UpdateInfo>('update://available', e => {
    if (ctx.state.kind === 'applying') return;
    render(ctx, { kind: 'available', info: e.payload });
  });

  void listen<string>('update://applying', e => {
    render(ctx, { kind: 'applying', version: e.payload });
  });

  // На mount — проверяем persistent applying_version. Если есть — сразу
  // в applying-режим (быстрее чем ждать emit от setup hook'а, который
  // может прийти после первого render'а).
  void invoke<string | null>('get_update_applying')
    .then(v => {
      if (v && ctx.state.kind === 'idle') {
        render(ctx, { kind: 'applying', version: v });
      }
    })
    .catch(() => { /* ignore */ });
}

export function mountUpdateBanner(): void {
  const refs = queryRefs();
  if (!refs) return;

  const ctx: Ctx = { refs, state: { kind: 'idle' }, installPollTimer: null };

  wireAvailableActions(ctx);
  wireApplyingActions(ctx);
  wireBannerSources(ctx);
}
