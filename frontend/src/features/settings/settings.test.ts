import { beforeEach, describe, expect, it, vi } from 'vitest';
import { mountSettings } from './settings';
import { getFormConfig, saveConfig } from './save-config';
import { refreshMicDevice } from './mic-device';
import { updateModelBadge } from '../../widgets/model-badge/model-badge';
import { initModelCardsHandlers } from '../model-management/model-cards';
import { settingsState, type SettingsConfigSnapshot } from './state';

// side-effect модули settings-страницы мокаем no-op'ами: тестируем оркестрацию
// mountSettings (регистрация handler'ов, change-detection, cancel/save), а не их
// внутренности. engine-availability / state / tauri / i18n — РЕАЛЬНЫЕ
// (preloadChangedEngines нужен checkChanges; state — простой синглтон).
vi.mock('./save-config', () => ({ getFormConfig: vi.fn(), saveConfig: vi.fn(async () => {}) }));
vi.mock('./mic-device', () => ({ refreshMicDevice: vi.fn(async () => {}) }));
vi.mock('./load-config', () => ({ loadSettings: vi.fn(async () => {}), setWidgetPositionPicker: vi.fn() }));
vi.mock('../../widgets/model-badge/model-badge', () => ({ updateModelBadge: vi.fn(async () => {}) }));
vi.mock('../model-management/model-cards', () => ({
  initModelCardsHandlers: vi.fn(),
  renderModelCards: vi.fn(async () => {}),
}));
vi.mock('../hotkey-config/hotkey-config', () => ({ initHotkeyComposer: vi.fn(), setHotkeyValue: vi.fn() }));
vi.mock('../page-navigation/page-navigation', () => ({ showPage: vi.fn() }));
vi.mock('../../shared/ui/toast', () => ({ showToast: vi.fn() }));
vi.mock('../../shared/ui/confirm', () => ({ showConfirm: vi.fn(async () => false) }));
vi.mock('../../shared/ui/custom-select', () => ({
  bindCustomSelect: vi.fn(),
  closeAllCustomSelects: vi.fn(),
  ensureGlobalCloseOnClick: vi.fn(),
  initCustomSelects: vi.fn(),
  setCustomSelectValue: vi.fn(),
}));

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockResolvedValue: (v: unknown) => void;
};

describe('mountSettings', () => {
  function baseConfig(over: Partial<SettingsConfigSnapshot> = {}): SettingsConfigSnapshot {
    return {
      transcriber: 'gigaam', model_path: '', whisper_model_path: '', gigaam_model_path: '',
      gigaam_rnnt_model_path: '', qwen3asr_model_path: '', sample_rate: 16000, max_record_secs: 60,
      hotkey: '', hotkey_pause: '', auto_type: false, debug: false, vad_enabled: false,
      vad_silence_secs: 3, remove_fillers: false, mic_gain: 1.0, mic_gain_per_device: {},
      retention_hours: 0, autostart: false, start_minimized: false, show_widget: true,
      widget_position: 'bottom-center', show_tray: true, models_base_dir: '', preload_models: [],
      ...over,
    };
  }

  // Toggle-строки, которые трогает cancel-restore (каждая — div с классом-состоянием .on).
  const TOGGLE_IDS = [
    's-auto-type', 's-debug', 's-vad-enabled', 's-remove-fillers', 's-autostart',
    's-start-minimized', 's-show-widget', 's-show-tray',
    's-preload-vosk', 's-preload-whisper', 's-preload-gigaam', 's-preload-gigaam-rnnt', 's-preload-qwen3asr',
  ];

  function toggle(id: string): void {
    const row = document.createElement('div');
    row.className = 'setting-row';
    const el = document.createElement('div');
    el.id = id;
    row.appendChild(el);
    document.body.appendChild(row);
  }

  function input(id: string): void {
    const el = document.createElement('input');
    el.id = id;
    document.body.appendChild(el);
  }

  // Прогнать очередь микротактов до конца (cancel/save-handler'ы имеют по
  // несколько await — restore/save завершается только после всех них).
  const flush = (): Promise<void> => new Promise(resolve => setTimeout(resolve, 0));

  // Полный DOM-фикстур settings-страницы: все элементы, к которым cancel/save/checkChanges
  // обращаются без `?.` (иначе null!.value / null!.classList кинет).
  function buildSettingsDom(): { saveBtn: HTMLButtonElement; cancelBtn: HTMLButtonElement; saveNotice: HTMLElement } {
    const saveBtn = document.createElement('button');
    saveBtn.id = 'save-btn';
    const cancelBtn = document.createElement('button');
    cancelBtn.id = 'cancel-btn';
    const saveNotice = document.createElement('div');
    saveNotice.id = 'save-notice';
    document.body.append(saveBtn, cancelBtn, saveNotice);
    for (const id of ['s-models-base-dir', 's-max-record', 's-vad-silence', 's-mic-gain']) input(id);
    for (const id of TOGGLE_IDS) toggle(id);
    return { saveBtn: saveBtn as HTMLButtonElement, cancelBtn: cancelBtn as HTMLButtonElement, saveNotice };
  }

  beforeEach(() => {
    settingsState.setOriginal(null);
    invokeMock.mockResolvedValue(false); // is_wayland / is_gnome / widget_extension_status → skip платформо-блоки
    vi.mocked(getFormConfig).mockReturnValue(baseConfig());
  });

  it('без обязательных кнопок — ранний return, обработчики не вешаются', () => {
    // Пустой DOM: нет save-btn/cancel-btn/save-notice.
    mountSettings();
    expect(vi.mocked(initModelCardsHandlers)).not.toHaveBeenCalled();
  });

  it('клик по toggle подсвечивает изменённое поле и активирует save', () => {
    const { saveBtn } = buildSettingsDom();
    settingsState.setOriginal(baseConfig({ auto_type: false }));
    // getFormConfig отражает «включённый» auto_type → checkChanges увидит расхождение.
    vi.mocked(getFormConfig).mockReturnValue(baseConfig({ auto_type: true }));
    mountSettings();

    const autoTypeEl = document.getElementById('s-auto-type')!;
    autoTypeEl.click();

    expect(autoTypeEl.classList.contains('on')).toBe(true);
    expect(saveBtn.disabled).toBe(false);
    expect(autoTypeEl.closest('.setting-row')!.classList.contains('changed')).toBe(true);
  });

  it('cancel восстанавливает toggle-состояния из original и рефрешит мик', async () => {
    const { saveBtn, cancelBtn } = buildSettingsDom();
    const orig = baseConfig({ auto_type: true, debug: true, preload_models: ['gigaam'] });
    settingsState.setOriginal(orig);
    // После restore getFormConfig прочитал бы восстановленную форму == orig → форма чистая.
    vi.mocked(getFormConfig).mockReturnValue(orig);
    mountSettings();

    cancelBtn.click();
    await flush(); // дать отработать async cancel-handler'у (два await внутри)

    expect(document.getElementById('s-auto-type')!.classList.contains('on')).toBe(true);
    expect(document.getElementById('s-debug')!.classList.contains('on')).toBe(true);
    expect(document.getElementById('s-preload-gigaam')!.classList.contains('on')).toBe(true);
    expect(document.getElementById('s-preload-vosk')!.classList.contains('on')).toBe(false);
    expect(vi.mocked(refreshMicDevice)).toHaveBeenCalled();
    // После восстановления форма не грязная (getFormConfig-мок == original по умолчанию).
    expect(saveBtn.disabled).toBe(true);
  });

  it('save сохраняет конфиг, показывает notice и обновляет badge', async () => {
    const { saveBtn, saveNotice } = buildSettingsDom();
    settingsState.setOriginal(baseConfig());
    vi.mocked(getFormConfig).mockReturnValue(baseConfig({ max_record_secs: 99 }));
    mountSettings();

    saveBtn.click();
    await flush();

    expect(vi.mocked(saveConfig)).toHaveBeenCalledWith(expect.objectContaining({ max_record_secs: 99 }));
    expect(saveNotice.classList.contains('visible')).toBe(true);
    expect(vi.mocked(updateModelBadge)).toHaveBeenCalled();
  });
});
