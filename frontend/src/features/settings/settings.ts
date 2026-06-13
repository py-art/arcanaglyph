// features/settings/settings.ts
//
// Главный entry settings-страницы. Регистрирует обработчики на форму,
// подписывается на нужные tray-команды, инициализирует custom-selects /
// hotkey-композеры / model-cards. Идемпотентен — должен зваться один раз
// при загрузке приложения (из app/init.ts).

import { invoke, listen } from '../../shared/lib/tauri';
import { i18n } from '../../shared/lib/i18n';
import { showToast } from '../../shared/ui/toast';
import { showConfirm } from '../../shared/ui/confirm';
import {
  bindCustomSelect,
  closeAllCustomSelects,
  ensureGlobalCloseOnClick,
  initCustomSelects,
  setCustomSelectValue,
} from '../../shared/ui/custom-select';
import { showPage } from '../page-navigation/page-navigation';
import { initHotkeyComposer, setHotkeyValue } from '../hotkey-config/hotkey-config';
import { initModelCardsHandlers, renderModelCards } from '../model-management/model-cards';
import { updateModelBadge } from '../../widgets/model-badge/model-badge';
import { applyEngineAvailability, preloadChangedEngines, updatePreloadLocks } from './engine-availability';
import { refreshMicDevice } from './mic-device';
import { loadSettings, setWidgetPositionPicker } from './load-config';
import { getFormConfig, saveConfig } from './save-config';
import { settingsState } from './state';

// Движки с отдельными preload-тумблерами (порядок = порядок строк в UI).
const PRELOAD_ENGINES = ['vosk', 'whisper', 'gigaam', 'qwen3asr'] as const;

// Маппинг полей конфига → DOM элементов для подсветки
const fieldMap: Record<string, { el: string; type: 'group' | 'row' }> = {
  transcriber:       { el: 's-transcriber',  type: 'group' },
  models_base_dir:   { el: 's-models-base-dir', type: 'group' },
  sample_rate:       { el: 's-sample-rate',  type: 'group' },
  max_record_secs:   { el: 's-max-record',   type: 'group' },
  hotkey:            { el: 'hk-trigger',      type: 'group' },
  hotkey_pause:      { el: 'hk-pause',        type: 'group' },
  auto_type:         { el: 's-auto-type',     type: 'row' },
  debug:             { el: 's-debug',          type: 'row' },
  vad_enabled:       { el: 's-vad-enabled',    type: 'row' },
  vad_silence_secs:  { el: 's-vad-silence',    type: 'group' },
  remove_fillers:    { el: 's-remove-fillers',  type: 'row' },
  // Изменения gain пишутся в mic_gain_per_device (не в global mic_gain).
  // Подсвечивается визуально на ползунке s-mic-gain.
  mic_gain_per_device: { el: 's-mic-gain',       type: 'group' },
  retention_hours:   { el: 's-retention',       type: 'group' },
  autostart:         { el: 's-autostart',        type: 'row' },
  start_minimized:   { el: 's-start-minimized', type: 'row' },
  show_widget:       { el: 's-show-widget',     type: 'row' },
  widget_position:   { el: 's-widget-position', type: 'group' },
  show_tray:         { el: 's-show-tray',       type: 'row' },
  // preload_models НЕ здесь: это один логический ключ, но 4 отдельных
  // тумблера-строки — подсветка изменений делается по-тумблерно в checkChanges
  // (модель «один ключ → один элемент» этого fieldMap для него не подходит).
};

export function mountSettings(): void {
  const saveBtn = document.getElementById('save-btn') as HTMLButtonElement | null;
  const cancelBtn = document.getElementById('cancel-btn') as HTMLButtonElement | null;
  const saveNotice = document.getElementById('save-notice');
  if (!saveBtn || !cancelBtn || !saveNotice) return;

  const resetSaveButtons = (): void => {
    saveBtn.disabled = true;
    saveNotice.classList.remove('visible');
  };

  // Проверка изменений + подсветка изменённых полей
  function checkChanges(): void {
    const orig = settingsState.getOriginal();
    if (!orig) return;
    const current = getFormConfig();
    let anyChanged = false;

    for (const [key, info] of Object.entries(fieldMap)) {
      const changed = JSON.stringify((current as any)[key]) !== JSON.stringify((orig as any)[key]);
      const el = document.getElementById(info.el);
      if (!el) continue;
      const container = info.type === 'group'
        ? el.closest('.setting-group')
        : el.closest('.setting-row');
      if (container) container.classList.toggle('changed', changed);
      if (changed) anyChanged = true;
    }

    // preload_models — один логический ключ, но 4 отдельных тумблера-строки.
    // Подсвечиваем КАЖДУЮ строку независимо (изменилось ли членство именно этого
    // движка), иначе .changed всегда вешалась бы на первую строку (s-preload-vosk).
    const curPreload: string[] = (current as any).preload_models || [];
    const origPreload: string[] = (orig as any).preload_models || [];
    const changedEngines = preloadChangedEngines(curPreload, origPreload, PRELOAD_ENGINES);
    for (const eng of PRELOAD_ENGINES) {
      const row = document.getElementById(`s-preload-${eng}`)?.closest('.setting-row');
      if (row) row.classList.toggle('changed', changedEngines.includes(eng));
    }
    if (changedEngines.length) anyChanged = true;

    saveBtn!.disabled = !anyChanged;
    cancelBtn!.disabled = !anyChanged;
    saveNotice!.classList.remove('visible');
  }

  const reloadSettings = async (): Promise<void> => {
    await loadSettings({ resetSaveButtons });
  };

  // Inject зависимости в model-cards (handlers download/delete + listen).
  initModelCardsHandlers({
    onChange: checkChanges,
    reloadSettings,
  });

  // Tray → settings + меню → settings
  void listen('tray://open-settings', async () => {
    showPage('settings');
    await reloadSettings();
  });

  document.getElementById('menu-settings')?.addEventListener('click', async () => {
    showPage('settings');
    await reloadSettings();
  });

  // === Custom-selects ===
  ensureGlobalCloseOnClick();
  initCustomSelects();

  // === Input/select handlers (change-detection) ===
  ['s-models-base-dir', 's-max-record', 's-vad-silence', 's-mic-gain']
    .forEach(id => document.getElementById(id)?.addEventListener('input', checkChanges));

  // Кнопка ↻ Обновить — пере-запрашивает активный микрофон у backend и
  // подставляет соответствующий override в ползунок. Полезно когда пользователь
  // переключает мик в системе (Bluetooth headset, USB, встроенный) без перезапуска UI.
  document.getElementById('s-mic-refresh')?.addEventListener('click', async () => {
    await refreshMicDevice(settingsState.getOriginal());
    checkChanges();  // ползунок мог поменять значение → обновить dirty-state
  });
  // Кастомные стрелки для number-input
  document.querySelectorAll<HTMLElement>('.number-input-wrap').forEach(wrap => {
    const input = wrap.querySelector<HTMLInputElement>('input[type="number"]');
    if (!input) return;
    wrap.querySelector('.arrow-up')?.addEventListener('click', () => {
      input.stepUp();
      input.dispatchEvent(new Event('input'));
    });
    wrap.querySelector('.arrow-down')?.addEventListener('click', () => {
      input.stepDown();
      input.dispatchEvent(new Event('input'));
    });
  });
  // Кнопка выбора базовой директории моделей
  document.getElementById('pick-base-dir')?.addEventListener('click', async () => {
    const baseInput = document.getElementById('s-models-base-dir') as HTMLInputElement;
    const currentPath = baseInput.value || '';
    const selected = await (window as any).__TAURI__.dialog.open({
      directory: true,
      title: i18n.t('model.pick_base_title'),
      defaultPath: currentPath,
    });
    if (selected) {
      baseInput.value = selected;
      checkChanges();
    }
  });
  ['s-transcriber', 's-sample-rate', 's-retention']
    .forEach(id => document.getElementById(id)?.addEventListener('change', checkChanges));
  // При смене модели по умолчанию — обновить блокировки предзагрузки + предупредить
  // если выбрана тяжёлая модель и у CPU нет AVX2 (Whisper Large без AVX2 крайне медленный).
  document.getElementById('s-transcriber')?.addEventListener('change', () => {
    const v = (document.getElementById('s-transcriber') as HTMLElement).dataset.value || '';
    updatePreloadLocks(v);
    const cpu = settingsState.getCpuFeatures();
    const heavyOnNoAvx2 =
      (v === 'whisper-large' || v === 'qwen3asr') &&
      cpu && cpu.avx2 === false;
    if (heavyOnNoAvx2) {
      showToast(i18n.t('settings.slow_model_warning'), 'warning', 7000);
    }
  });

  // Смена языка — применяется сразу и сохраняется автоматически
  bindCustomSelect('s-language', {
    onChange: async lang => {
      i18n.setLanguage(lang as any);
      try {
        await invoke('set_language', { lang });
        showToast(i18n.t('toast.saved'), 'success', 1500);
      } catch (e) {
        showToast(`${i18n.t('toast.save_error')}: ${e}`, 'error', 3000);
      }
    },
  });

  // === Hotkey composers ===
  initHotkeyComposer('hk-trigger', checkChanges);
  initHotkeyComposer('hk-pause', checkChanges);

  // Проверяем Wayland и показываем предупреждение
  void invoke<boolean>('is_wayland').then(wayland => {
    if (wayland) {
      const w = document.getElementById('wayland-warning');
      if (w) w.style.display = 'block';
      // Хинт под picker'ом — mutter может проигнорировать выбор позиции
      const hint = document.getElementById('s-widget-position-hint');
      if (hint) hint.hidden = false;
    }
  });

  // GNOME-Wayland: показываем toggle GNOME-расширения для точного позиционирования.
  // На X11 / KDE / sway / Cinnamon — расширение не применимо, ряд скрыт.
  Promise.all([
    invoke<boolean>('is_wayland'),
    invoke<boolean>('is_gnome'),
    invoke<{ available: boolean; enabled: boolean }>('widget_extension_status'),
  ])
    .then(([wayland, gnome, status]) => {
      if (!(wayland && gnome && status.available)) return;
      const row = document.getElementById('s-widget-ext-row');
      const hint = document.getElementById('s-widget-ext-hint');
      const ext = document.getElementById('s-widget-ext');
      if (row) row.hidden = false;
      if (hint) hint.hidden = false;
      // Текущее состояние toggle = enabled в gsettings (status.enabled).
      // Если установлено но не enabled — toggle off (пользователь выключил вручную).
      if (ext) ext.classList.toggle('on', status.enabled);
    }).catch(() => { /* gsettings/non-GNOME — игнорируем */ });

  // Click-handler на toggle расширения: install + enable + предложить logout, либо disable.
  document.getElementById('s-widget-ext')?.addEventListener('click', async () => {
    const toggle = document.getElementById('s-widget-ext')!;
    const wantOn = !toggle.classList.contains('on');
    // Оптимистично переключаем визуально; откатим если бэкенд вернёт ошибку.
    toggle.classList.toggle('on', wantOn);
    try {
      if (wantOn) {
        // backend возвращает true если уже было включено — тогда не предлагаем relogin
        const wasAlreadyEnabled = await invoke<boolean>('install_widget_extension');
        if (wasAlreadyEnabled) {
          showToast(i18n.t('toast.widget_ext_already_enabled'), 'success', 3000);
        } else {
          showToast(i18n.t('toast.widget_ext_installed'), 'success', 4000);
          const wantsLogout = await showConfirm(
            i18n.t('modal.widget_ext_logout_question'),
            i18n.t('modal.logout_now'),
            i18n.t('modal.later'),
          );
          if (wantsLogout) {
            await invoke('request_logout').catch(() => {});
          }
        }
      } else {
        await invoke('disable_widget_extension');
        showToast(i18n.t('toast.widget_ext_disabled'), 'success', 3000);
      }
    } catch (e) {
      // Откатываем визуально и сообщаем об ошибке
      toggle.classList.toggle('on', !wantOn);
      showToast(`${i18n.t('toast.error')}: ${e}`, 'error', 5000);
    }
  });

  // Picker позиции виджета: клик по точке выставляет её активной + триггерит changed
  document.querySelectorAll<HTMLElement>('#s-widget-position .position-picker-dot').forEach(dot => {
    dot.addEventListener('click', () => {
      setWidgetPositionPicker(dot.dataset.pos || 'bottom-center');
      checkChanges();
    });
  });

  // Переключение табов настроек
  document.querySelectorAll<HTMLElement>('.settings-tab').forEach(tab => {
    tab.addEventListener('click', () => {
      document.querySelectorAll('.settings-tab').forEach(t => t.classList.remove('active'));
      document.querySelectorAll('.settings-tab-content').forEach(c => c.classList.remove('active'));
      tab.classList.add('active');
      document.getElementById('tab-' + tab.dataset.tab)?.classList.add('active');
      closeAllCustomSelects();
    });
  });

  // Toggle-переключатели
  ['s-auto-type', 's-debug', 's-vad-enabled', 's-remove-fillers',
   's-autostart', 's-start-minimized', 's-show-tray', 's-show-widget',
   's-preload-vosk', 's-preload-whisper', 's-preload-gigaam', 's-preload-qwen3asr',
  ].forEach(id => {
    const el = document.getElementById(id);
    if (!el) return;
    el.addEventListener('click', () => {
      el.classList.toggle('on');
      checkChanges();
    });
  });

  // Отмена изменений — восстанавливаем оригинальные значения.
  // Async чтобы дождаться refreshMicDevice (он делает invoke get_default_input_device_name).
  // Без await checkChanges() видел старое значение ползунка и не сбрасывал dirty-state
  // с первого клика — приходилось жать дважды.
  cancelBtn.addEventListener('click', async () => {
    const orig = settingsState.getOriginal();
    if (!orig) return;
    setCustomSelectValue('s-transcriber', orig.transcriber);
    (document.getElementById('s-models-base-dir') as HTMLInputElement).value = orig.models_base_dir || '';
    await renderModelCards();
    setCustomSelectValue('s-sample-rate', String(orig.sample_rate));
    (document.getElementById('s-max-record') as HTMLInputElement).value = String(orig.max_record_secs);
    setHotkeyValue('hk-trigger', orig.hotkey);
    setHotkeyValue('hk-pause', orig.hotkey_pause || '');
    document.getElementById('s-auto-type')!.classList.toggle('on', orig.auto_type);
    document.getElementById('s-debug')!.classList.toggle('on', orig.debug);
    document.getElementById('s-vad-enabled')!.classList.toggle('on', orig.vad_enabled !== false);
    (document.getElementById('s-vad-silence') as HTMLInputElement).value = String(orig.vad_silence_secs || 7);
    document.getElementById('s-remove-fillers')!.classList.toggle('on', orig.remove_fillers !== false);
    // Ждём пока refreshMicDevice реально обновит ползунок ДО checkChanges.
    await refreshMicDevice(orig);
    setCustomSelectValue('s-retention', String(orig.retention_hours || 0));
    document.getElementById('s-autostart')!.classList.toggle('on', orig.autostart || false);
    document.getElementById('s-start-minimized')!.classList.toggle('on', orig.start_minimized || false);
    document.getElementById('s-show-widget')!.classList.toggle('on', orig.show_widget !== false);
    setWidgetPositionPicker(orig.widget_position || 'bottom-center');
    document.getElementById('s-show-tray')!.classList.toggle('on', orig.show_tray !== false);
    const preloadOrig = orig.preload_models || [];
    document.getElementById('s-preload-vosk')!.classList.toggle('on', preloadOrig.includes('vosk'));
    document.getElementById('s-preload-whisper')!.classList.toggle('on', preloadOrig.includes('whisper'));
    document.getElementById('s-preload-gigaam')!.classList.toggle('on', preloadOrig.includes('gigaam'));
    document.getElementById('s-preload-qwen3asr')!.classList.toggle('on', preloadOrig.includes('qwen3asr'));
    checkChanges();
  });

  // Сохранение
  saveBtn.addEventListener('click', async () => {
    const cfg = getFormConfig();
    await saveConfig(cfg);
    settingsState.setOriginal(JSON.parse(JSON.stringify(cfg)));
    checkChanges();
    saveNotice.classList.add('visible');
    void updateModelBadge();
  });
}

// Re-export для использования в init.ts (запрос availability при старте)
export { applyEngineAvailability };
