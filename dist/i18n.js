// dist/i18n.js
// Локализация интерфейса: русский (ru) и английский (en).
// Использование:
//   - Статические элементы: атрибуты data-i18n, data-i18n-title, data-i18n-placeholder
//   - Динамические строки: i18n.t('key.path', {param: value})
//   - Смена языка: i18n.setLanguage('en') → автоматически пересобирает тексты

(function () {
  const DICTS = {
    ru: {
      status: {
        loading: 'Загрузка модели...',
        loading_model: 'Загрузка модели {model}…',
        downloading_model: 'Скачивается модель: {percent}%',
        ready: 'Готов к записи',
        recording: 'Запись...',
        paused: 'Пауза',
        transcribing: 'Транскрибация...',
      },
      titlebar: {
        minimize: 'Свернуть',
        maximize: 'Развернуть',
        close: 'Закрыть',
        back: '← Назад',
      },
      mic: {
        pause: 'Пауза',
        resume: 'Продолжить',
        stop: 'Остановить',
        copy: 'Копировать',
      },
      menu: {
        title: 'Меню',
        settings: 'Настройки',
        history: 'История',
        about: 'О приложении',
      },
      about: {
        desc: 'Голосовой ввод текста',
        sub1: 'Нажмите горячую клавишу — говорите —',
        sub2: 'нажмите ещё раз — текст вставится в активное окно',
        privacy: 'Вся транскрибация — локально, без облака',
      },
      history: {
        title: 'История',
        period: 'Период',
        empty: 'Записей нет',
        prev: '← Пред',
        next: 'След →',
        clear: 'Очистить историю',
        export: 'Экспорт',
        page: 'Стр. {current} из {total}',
        recognize_btn: 'Распознать этой моделью',
        recognizing: 'Распознавание...',
        not_recognized: '(не распознано)',
        audio_deleted: 'Аудиофайл удалён',
        play: 'Воспроизвести',
        copy: 'Копировать',
        delete: 'Удалить',
        confirm_clear: 'Удалить всю историю?',
      },
      period: {
        '30min': '30 минут',
        '1h': '1 час',
        '12h': '12 часов',
        '24h': '24 часа',
        '2d': '2 дня',
        '7d': '7 дней',
        '15d': '15 дней',
        '30d': '30 дней',
        '2m': '2 месяца',
        '6m': '6 месяцев',
        '1y': '1 год',
        all: 'Все записи',
      },
      settings: {
        title: 'Настройки',
        tabs: { general: 'Основное', models: 'Модели', hotkeys: 'Клавиши' },
        transcriber: 'Движок транскрибации',
        t_vosk: 'Vosk — быстрый, менее точный',
        t_whisper: 'Whisper — точный, медленнее',
        t_whisper_tiny: 'Whisper Tiny — для слабых CPU',
        t_whisper_large: 'Whisper Large V3 Turbo — точнее, нужен AVX2',
        t_gigaam: 'GigaAM v3 — лучший для русского',
        t_qwen3asr: 'Qwen3-ASR — мультиязычный',
        slow_model_warning: 'На вашем CPU нет AVX2 — эта модель будет работать в 10-30× медленнее обычного, и прервать транскрибацию можно только убийством приложения. Лучше GigaAM v3 или Whisper Tiny.',
        engine_unavailable: 'не собрано',
        model_not_installed: 'нет модели',
        preload_vosk: 'Предзагрузка Vosk',
        preload_whisper: 'Предзагрузка Whisper',
        preload_gigaam: 'Предзагрузка GigaAM',
        preload_qwen3asr: 'Предзагрузка Qwen3-ASR',
        sample_rate: 'Частота дискретизации (Гц)',
        max_record: 'Таймаут записи (секунды)',
        vad_enabled: 'Авто-стоп при тишине',
        vad_silence: 'Тишина для авто-стопа (секунды)',
        retention: 'Автоочистка записей',
        retention_never: 'Хранить вечно',
        retention_1h: '1 час',
        retention_12h: '12 часов',
        retention_1d: '1 день',
        retention_1w: '1 неделя',
        retention_30d: '30 дней',
        retention_6m: 'Полгода',
        retention_1y: '1 год',
        auto_type: 'Авто-вставка текста',
        debug: 'Режим отладки',
        remove_fillers: 'Удалять слова-паразиты',
        mic_gain: 'Усиление микрофона',
        mic_gain_hint: '1.0 = без усиления, 2.0 = +6 дБ, 5.0 = +14 дБ. Сохраняется отдельно для каждого микрофона.',
        mic_refresh: 'Обновить активный микрофон',
        autostart: 'Автозапуск при входе',
        start_minimized: 'Запуск в трей',
        show_tray: 'Иконка в трее',
        show_widget: 'Виджет записи',
        language: 'Язык интерфейса',
        lang_ru: 'Русский',
        lang_en: 'English',
        hotkey_trigger: 'Запись (старт/стоп)',
        hotkey_pause: 'Пауза',
        hotkey_key_placeholder: 'клавиша',
        hotkey_record_title: 'Записать клавишу',
        hotkey_clear_title: 'Очистить',
        wayland_warning:
          'Wayland: горячие клавиши регистрируются через GNOME Settings (gsettings). При сохранении комбинации автоматически применяются в системе.',
        models_base_dir: 'Базовый путь к моделям',
        models_title: 'Доступные модели',
        cancel: 'Отменить',
        save: 'Сохранить',
        save_notice: 'Настройки сохранены. Применятся при следующей записи.',
        pick_base_title: 'Выбрать директорию',
      },
      model: {
        installed: 'Установлена',
        missing: 'Не найдена',
        not_available: 'Недоступна в этой сборке',
        download: 'Скачать',
        downloading: 'Скачивание...',
        extracting: 'Распаковка...',
        delete: 'Удалить',
        delete_confirm: 'Удалить модель {name} с диска? ({size})',
        deleted: 'Модель удалена',
        delete_error: 'Не удалось удалить',
        load_error: 'Не удалось загрузить список моделей',
        pick_placeholder: 'Нажмите 📁 для выбора',
        pick_dir_title: 'Выбрать директорию модели',
        pick_base_title: 'Выбрать базовую директорию моделей',
        download_error: 'Ошибка скачивания',
      },
      modal: {
        cancel: 'Отмена',
        confirm_delete: 'Удалить',
      },
      toast: {
        saved: 'Сохранено',
        save_error: 'Ошибка сохранения',
        file_saved: 'Файл сохранён в папку Загрузки',
        error: 'Ошибка',
        engine_fallback: 'Движок «{original}» не включён в сборку — используется «{fallback}»',
      },
      result: {
        empty: '(пустой результат)',
        unknown_error: 'Неизвестная ошибка',
      },
      hotkey: {
        trigger_conflict: 'Trigger ({hotkey}): занята — {holder}',
        pause_conflict: 'Pause ({hotkey}): занята — {holder}',
        conflicts_prompt: 'Обнаружены конфликты клавиш:\n\n{list}\n\nВсё равно сохранить?',
      },
    },
    en: {
      status: {
        loading: 'Loading model...',
        loading_model: 'Loading model {model}…',
        downloading_model: 'Downloading model: {percent}%',
        ready: 'Ready to record',
        recording: 'Recording...',
        paused: 'Paused',
        transcribing: 'Transcribing...',
      },
      titlebar: {
        minimize: 'Minimize',
        maximize: 'Maximize',
        close: 'Close',
        back: '← Back',
      },
      mic: {
        pause: 'Pause',
        resume: 'Resume',
        stop: 'Stop',
        copy: 'Copy',
      },
      menu: {
        title: 'Menu',
        settings: 'Settings',
        history: 'History',
        about: 'About',
      },
      about: {
        desc: 'Voice text input',
        sub1: 'Press the hotkey — speak —',
        sub2: 'press again — text is inserted into the active window',
        privacy: 'All transcription runs locally, no cloud',
      },
      history: {
        title: 'History',
        period: 'Period',
        empty: 'No records',
        prev: '← Prev',
        next: 'Next →',
        clear: 'Clear history',
        export: 'Export',
        page: 'Page {current} of {total}',
        recognize_btn: 'Recognize with this model',
        recognizing: 'Recognizing...',
        not_recognized: '(not recognized)',
        audio_deleted: 'Audio file deleted',
        play: 'Play',
        copy: 'Copy',
        delete: 'Delete',
        confirm_clear: 'Delete all history?',
      },
      period: {
        '30min': '30 minutes',
        '1h': '1 hour',
        '12h': '12 hours',
        '24h': '24 hours',
        '2d': '2 days',
        '7d': '7 days',
        '15d': '15 days',
        '30d': '30 days',
        '2m': '2 months',
        '6m': '6 months',
        '1y': '1 year',
        all: 'All records',
      },
      settings: {
        title: 'Settings',
        tabs: { general: 'General', models: 'Models', hotkeys: 'Hotkeys' },
        transcriber: 'Transcription engine',
        t_vosk: 'Vosk — fast, less accurate',
        t_whisper: 'Whisper — accurate, slower',
        t_whisper_tiny: 'Whisper Tiny — for slow CPUs',
        t_whisper_large: 'Whisper Large V3 Turbo — better, needs AVX2',
        slow_model_warning: 'Your CPU lacks AVX2 — this model will be 10-30× slower than usual, and the only way to interrupt transcription is to kill the app. Prefer GigaAM v3 or Whisper Tiny.',
        t_gigaam: 'GigaAM v3 — best for Russian',
        t_qwen3asr: 'Qwen3-ASR — multilingual',
        engine_unavailable: 'not built',
        model_not_installed: 'no model',
        preload_vosk: 'Preload Vosk',
        preload_whisper: 'Preload Whisper',
        preload_gigaam: 'Preload GigaAM',
        preload_qwen3asr: 'Preload Qwen3-ASR',
        sample_rate: 'Sample rate (Hz)',
        max_record: 'Recording timeout (seconds)',
        vad_enabled: 'Auto-stop on silence',
        vad_silence: 'Silence for auto-stop (seconds)',
        retention: 'Auto-cleanup records',
        retention_never: 'Keep forever',
        retention_1h: '1 hour',
        retention_12h: '12 hours',
        retention_1d: '1 day',
        retention_1w: '1 week',
        retention_30d: '30 days',
        retention_6m: '6 months',
        retention_1y: '1 year',
        auto_type: 'Auto-insert text',
        debug: 'Debug mode',
        remove_fillers: 'Remove filler words',
        mic_gain: 'Microphone gain',
        mic_gain_hint: '1.0 = no boost, 2.0 = +6 dB, 5.0 = +14 dB. Saved separately per microphone.',
        mic_refresh: 'Refresh active microphone',
        autostart: 'Start on login',
        start_minimized: 'Start in tray',
        show_tray: 'Tray icon',
        show_widget: 'Recording widget',
        language: 'Interface language',
        lang_ru: 'Русский',
        lang_en: 'English',
        hotkey_trigger: 'Record (start/stop)',
        hotkey_pause: 'Pause',
        hotkey_key_placeholder: 'key',
        hotkey_record_title: 'Record key',
        hotkey_clear_title: 'Clear',
        wayland_warning:
          'Wayland: hotkeys are registered via GNOME Settings (gsettings). They apply to the system automatically when saved.',
        models_base_dir: 'Base path for models',
        models_title: 'Available models',
        cancel: 'Cancel',
        save: 'Save',
        save_notice: 'Settings saved. Will apply to the next recording.',
        pick_base_title: 'Pick directory',
      },
      model: {
        installed: 'Installed',
        missing: 'Not found',
        not_available: 'Not available in this build',
        download: 'Download',
        downloading: 'Downloading...',
        extracting: 'Extracting...',
        delete: 'Delete',
        delete_confirm: 'Delete model {name} from disk? ({size})',
        deleted: 'Model deleted',
        delete_error: 'Failed to delete',
        load_error: 'Failed to load model list',
        pick_placeholder: 'Click 📁 to pick',
        pick_dir_title: 'Pick model directory',
        pick_base_title: 'Pick base model directory',
        download_error: 'Download error',
      },
      modal: {
        cancel: 'Cancel',
        confirm_delete: 'Delete',
      },
      toast: {
        saved: 'Saved',
        save_error: 'Save error',
        file_saved: 'File saved to Downloads folder',
        error: 'Error',
        engine_fallback: 'Engine "{original}" is not enabled in this build — using "{fallback}"',
      },
      result: {
        empty: '(empty result)',
        unknown_error: 'Unknown error',
      },
      hotkey: {
        trigger_conflict: 'Trigger ({hotkey}): taken — {holder}',
        pause_conflict: 'Pause ({hotkey}): taken — {holder}',
        conflicts_prompt: 'Hotkey conflicts detected:\n\n{list}\n\nSave anyway?',
      },
    },
  };

  function detectDefault() {
    const locale = (navigator.language || 'ru').toLowerCase();
    return locale.startsWith('ru') ? 'ru' : 'en';
  }

  let currentLang = detectDefault();

  function lookup(path, lang) {
    const parts = path.split('.');
    let node = DICTS[lang] || DICTS.ru;
    for (const part of parts) {
      if (node == null) return null;
      node = node[part];
    }
    return typeof node === 'string' ? node : null;
  }

  function interpolate(str, params) {
    if (!params) return str;
    return str.replace(/\{(\w+)\}/g, (_, k) => (k in params ? params[k] : `{${k}}`));
  }

  function t(key, params) {
    const val = lookup(key, currentLang) || lookup(key, 'ru') || key;
    return interpolate(val, params);
  }

  function applyI18n(root) {
    const scope = root || document;
    scope.querySelectorAll('[data-i18n]').forEach((el) => {
      const key = el.dataset.i18n;
      el.textContent = t(key);
    });
    scope.querySelectorAll('[data-i18n-title]').forEach((el) => {
      const key = el.dataset.i18nTitle;
      el.title = t(key);
    });
    scope.querySelectorAll('[data-i18n-placeholder]').forEach((el) => {
      const key = el.dataset.i18nPlaceholder;
      el.placeholder = t(key);
    });

    // Обновляем триггер для уже открытых custom-select (чтобы показать перевод выбранной опции)
    scope.querySelectorAll('.custom-select').forEach((sel) => {
      const val = sel.dataset.value;
      if (val == null) return;
      const opt = sel.querySelector(`.custom-select-option[data-value="${CSS.escape(val)}"]`);
      const trigger = sel.querySelector('.custom-select-trigger');
      if (opt && trigger && !trigger.dataset.i18nCustom) {
        trigger.textContent = opt.textContent;
      }
    });
  }

  function setLanguage(lang) {
    if (!DICTS[lang]) lang = 'ru';
    currentLang = lang;
    document.documentElement.lang = lang;
    applyI18n();
  }

  function getLanguage() {
    return currentLang;
  }

  window.i18n = { t, applyI18n, setLanguage, getLanguage };
})();
