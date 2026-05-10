// @ts-nocheck
// FSD-migration in progress. Header импортирует FSD-слои; всё ниже —
// legacy inline-блок (settings / model-management / history) с уже-старой
// inline-структурой. Эти блоки extract-нуть в `features/*` — задача
// следующей итерации; они тесно переплетены state'ом (originalConfig,
// custom-select bindings, model card refs), безопасный split требует
// предварительного refactoring state-management.

import { initApp } from './app/init';
import { invoke, listen, appWindow } from './shared/lib/tauri';
import { i18n, t } from './shared/lib/i18n';
import { showToast } from './shared/ui/toast';
import { showConfirm } from './shared/ui/confirm';
import { updateModelBadge } from './widgets/model-badge/model-badge';
import { MODEL_SHORT_NAMES } from './entities/model/types';

// Bootstrap UI — монтирует titlebar / banners / main-controls / about.
const { onModelReady } = initApp();

// === Legacy inline-блок ниже ===
// Зависит от globals выше (invoke, listen, i18n, showToast, showConfirm,
// updateModelBadge, MODEL_SHORT_NAMES, onModelReady).
      listen('tray://open-settings', async () => {
        showPage('settings');
        await loadSettings();
      });

      /* ──────── Меню и настройки ──────── */
      const menuBtn     = document.getElementById('menu-btn');
      const menuPage    = document.getElementById('menu-page');
      const settingsPage = document.getElementById('settings-page');
      const contentEl   = document.querySelector('.content');
      const saveBtn     = document.getElementById('save-btn');
      const cancelBtn   = document.getElementById('cancel-btn');
      const saveNotice  = document.getElementById('save-notice');

      // Page navigation: showPage и currentPage здесь, потому что
      // history-блок ниже делает `showPage = function(...)` reassignment.
      // Feature-stub в `features/page-navigation/` есть, но не подключён —
      // ESM import-binding immutable, нельзя пере-присвоить. Перенос
      // отложен до refactoring history (заменить reassignment на
      // event-based extension).
      let currentPage = 'main';
      function showPage(page) {
        contentEl.style.display = page === 'main' ? '' : 'none';
        menuPage.classList.toggle('visible', page === 'menu');
        settingsPage.classList.toggle('visible', page === 'settings');
        menuBtn.classList.toggle('back', page !== 'main');
        currentPage = page;
      }

      let originalConfig = null;
      // CPU SIMD-фичи (AVX, AVX2, FMA). Заполняется один раз через `get_cpu_features`
      // при загрузке настроек, используется для warning-toast'а при выборе тяжёлой
      // модели на безAVX2-CPU.
      let cpuFeatures = null;


      // Обработчик меню переопределяется ниже (в секции Истории)

      document.getElementById('menu-settings').addEventListener('click', async () => {
        showPage('settings');
        await loadSettings();
      });

      // Установить значение кастомного select
      function setCustomSelect(id, value) {
        const sel = document.getElementById(id);
        sel.dataset.value = value;
        const opt = sel.querySelector(`[data-value="${value}"]`);
        const trigger = sel.querySelector('.custom-select-trigger');
        if (opt) {
          trigger.textContent = opt.textContent;
          sel.querySelectorAll('.custom-select-option').forEach(o => o.classList.remove('selected'));
          opt.classList.add('selected');
        }
      }

      // Picker позиции виджета: переставляет .active на нужную точку и обновляет data-value
      function setWidgetPositionPicker(pos) {
        const picker = document.getElementById('s-widget-position');
        if (!picker) return;
        picker.dataset.value = pos;
        picker.querySelectorAll('.position-picker-dot').forEach(d => {
          d.classList.toggle('active', d.dataset.pos === pos);
        });
      }

      // Нормализовать preload_models — модель по умолчанию всегда включена
      function normalizePreload(preload, transcriber) {
        const set = new Set(preload);
        set.add(transcriber);
        return [...set].sort();
      }

      // Блокировка toggle предзагрузки для модели по умолчанию
      // Маппит расщеплённое значение dropdown'a ('whisper-tiny' / 'whisper-large')
      // в обобщённый тип движка для логики, которая не различает варианты модели.
      function normalizeTranscriber(v) {
        if (v === 'whisper-tiny' || v === 'whisper-large') return 'whisper';
        return v;
      }

      function updatePreloadLocks(transcriber) {
        transcriber = normalizeTranscriber(transcriber);
        const voskToggle = document.getElementById('s-preload-vosk');
        const whisperToggle = document.getElementById('s-preload-whisper');
        const gigaamToggle = document.getElementById('s-preload-gigaam');
        const qwen3asrToggle = document.getElementById('s-preload-qwen3asr');
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

      // Помечает опции в dropdown'е «Движок транскрибации» как disabled при двух условиях:
      //   (1) движок не включён в cargo-сборку — лейбл «(не собрано)»
      //   (2) движок собран, но соответствующей модели нет на диске — лейбл «(нет модели)»
      // Опция остаётся видна (для информирования), но click не реагирует благодаря
      // CSS pointer-events: none + JS-проверке в опции (см. initCustomSelects).
      async function applyEngineAvailability() {
        let compiled = [];
        let models = [];
        try {
          compiled = await invoke('get_compiled_engines') || [];
          models = await invoke('get_models') || [];
        } catch (_) { /* старая сборка без команды — все опции активны */ }
        if (!Array.isArray(compiled) || compiled.length === 0) return;
        const sel = document.getElementById('s-transcriber');
        if (!sel) return;
        const notBuiltLabel = i18n.t('settings.engine_unavailable');
        const noModelLabel = i18n.t('settings.model_not_installed');

        // Карта dropdown-value → id модели в реестре. Whisper-варианты явные.
        // Для остальных движков — id первой найденной модели этого transcriber_type.
        const valueToModelId = (value) => {
          if (value === 'whisper-tiny') return 'whisper-tiny';
          if (value === 'whisper-large') return 'whisper-large-v3-turbo';
          // Для vosk/gigaam/qwen3asr берём первую модель с подходящим transcriber_type.
          const m = models.find(mm => mm.transcriber_type === value);
          return m ? m.id : null;
        };
        const isInstalled = (modelId) => {
          if (!modelId) return false;
          const m = models.find(mm => mm.id === modelId);
          return !!(m && m.installed);
        };

        sel.querySelectorAll('.custom-select-option').forEach(opt => {
          const dropdownValue = opt.dataset.value;
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

      // Загрузка настроек из бэкенда
      async function loadSettings() {
        const cfg = await invoke('load_config');
        const lang = cfg.language || i18n.getLanguage();
        setCustomSelect('s-language', lang);
        await applyEngineAvailability();
        // CPU-фичи нужны для warning-toast'а при выборе тяжёлой модели; берём один раз.
        if (!cpuFeatures) {
          try { cpuFeatures = await invoke('get_cpu_features'); }
          catch (e) { cpuFeatures = { avx: true, avx2: true, fma: true }; }
        }
        // Нормализуем типы чтобы совпадали с getFormConfig()
        originalConfig = {
          transcriber: cfg.transcriber,
          model_path: cfg.model_path,
          whisper_model_path: cfg.whisper_model_path,
          gigaam_model_path: cfg.gigaam_model_path,
          qwen3asr_model_path: cfg.qwen3asr_model_path,
          sample_rate: cfg.sample_rate,
          max_record_secs: cfg.max_record_secs,
          hotkey: cfg.hotkey,
          hotkey_pause: cfg.hotkey_pause || '',
          auto_type: cfg.auto_type,
          debug: cfg.debug,
          vad_enabled: cfg.vad_enabled !== false,
          vad_silence_secs: cfg.vad_silence_secs || 7,
          remove_fillers: cfg.remove_fillers !== false,
          mic_gain: typeof cfg.mic_gain === 'number' ? cfg.mic_gain : 1.0,
          mic_gain_per_device: cfg.mic_gain_per_device && typeof cfg.mic_gain_per_device === 'object' ? { ...cfg.mic_gain_per_device } : {},
          retention_hours: cfg.retention_hours || 0,
          autostart: cfg.autostart || false,
          start_minimized: cfg.start_minimized || false,
          show_widget: cfg.show_widget !== false,
          widget_position: cfg.widget_position || 'bottom-center',
          show_tray: cfg.show_tray !== false,
          models_base_dir: cfg.models_base_dir || '',
          preload_models: normalizePreload(cfg.preload_models || [], cfg.transcriber),
        };

        // Whisper расщеплён в dropdown'e на Tiny / Large V3 Turbo. Конфиг хранит
        // обобщённый transcriber="whisper" + whisper_model_path, поэтому при загрузке
        // определяем нужный пункт по имени файла модели.
        let dropdownValue = cfg.transcriber;
        if (cfg.transcriber === 'whisper') {
          const path = (cfg.whisper_model_path || '').toLowerCase();
          dropdownValue = path.includes('tiny') ? 'whisper-tiny' : 'whisper-large';
        }
        setCustomSelect('s-transcriber', dropdownValue);
        document.getElementById('s-models-base-dir').value = cfg.models_base_dir || '';
        setCustomSelect('s-sample-rate', String(cfg.sample_rate));
        document.getElementById('s-max-record').value = cfg.max_record_secs;
        setHotkeyValue('hk-trigger', cfg.hotkey);
        setHotkeyValue('hk-pause', cfg.hotkey_pause || '');
        document.getElementById('s-auto-type').classList.toggle('on', cfg.auto_type);
        document.getElementById('s-debug').classList.toggle('on', cfg.debug);
        document.getElementById('s-vad-enabled').classList.toggle('on', cfg.vad_enabled !== false);
        document.getElementById('s-vad-silence').value = cfg.vad_silence_secs || 3;
        document.getElementById('s-remove-fillers').classList.toggle('on', cfg.remove_fillers !== false);
        // Заполнение mic_gain через async refresh (берёт активное устройство и его override)
        refreshMicDevice(cfg);
        setCustomSelect('s-retention', String(cfg.retention_hours || 0));
        document.getElementById('s-autostart').classList.toggle('on', cfg.autostart || false);
        document.getElementById('s-start-minimized').classList.toggle('on', cfg.start_minimized || false);
        document.getElementById('s-show-widget').classList.toggle('on', cfg.show_widget !== false);
        setWidgetPositionPicker(cfg.widget_position || 'bottom-center');
        document.getElementById('s-show-tray').classList.toggle('on', cfg.show_tray !== false);
        const preload = cfg.preload_models || [];
        document.getElementById('s-preload-vosk').classList.toggle('on', preload.includes('vosk') || cfg.transcriber === 'vosk');
        document.getElementById('s-preload-whisper').classList.toggle('on', preload.includes('whisper') || cfg.transcriber === 'whisper');
        document.getElementById('s-preload-gigaam').classList.toggle('on', preload.includes('gigaam') || cfg.transcriber === 'gigaam');
        document.getElementById('s-preload-qwen3asr').classList.toggle('on', preload.includes('qwen3asr') || cfg.transcriber === 'qwen3asr');
        updatePreloadLocks(cfg.transcriber);

        saveBtn.disabled = true;
        saveNotice.classList.remove('visible');

        renderModelCards();
      }

      // Рендер карточек моделей на табе «Модели»
      async function renderModelCards() {
        const container = document.getElementById('model-cards-container');
        try {
          const models = await invoke('get_models');
          container.innerHTML = '';
          for (const m of models) {
            const card = document.createElement('div');
            card.className = 'model-card' + (m.available === false ? ' unavailable' : '');
            card.dataset.modelId = m.id;
            // Для недоступных движков в шапке через ::after подставляется метка.
            const unavailLabel = m.available === false ? i18n.t('model.not_available') : '';
            // Статус карточки: установлена / не установлена / не доступна (backend не собран)
            let statusCls, statusKey, statusText;
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
          container.querySelectorAll('.model-path-pick').forEach(btn => {
            btn.addEventListener('click', async () => {
              const input = btn.parentElement.querySelector('.model-path-input');
              const currentPath = input.value || document.getElementById('s-models-base-dir').value || '';
              const selected = await window.__TAURI__.dialog.open({
                directory: true,
                title: i18n.t('model.pick_dir_title'),
                defaultPath: currentPath,
              });
              if (selected) {
                input.value = selected;
                checkChanges();
              }
            });
          });
          // Отслеживание изменений в путях карточек
          container.querySelectorAll('.model-path-input').forEach(input => {
            input.addEventListener('input', checkChanges);
          });
          // Обработчики кнопок «Скачать»
          container.querySelectorAll('.model-download-btn').forEach(btn => {
            btn.addEventListener('click', async () => {
              const modelId = btn.dataset.id;
              const url = btn.dataset.url;
              const filename = btn.dataset.filename;
              const ttype = btn.dataset.type;

              // Определяем директорию скачивания из пути в карточке
              const card = btn.closest('.model-card');
              const pathInput = card.querySelector('.model-path-input');
              let destDir = pathInput ? pathInput.value : '';
              if (!destDir) destDir = document.getElementById('s-models-base-dir').value + '/' + filename;
              // Для whisper путь — к файлу, берём директорию
              if (ttype === 'whisper') {
                destDir = destDir.substring(0, destDir.lastIndexOf('/') + 1) || destDir;
              }

              btn.disabled = true;
              btn.textContent = i18n.t('model.downloading');
              btn.dataset.i18n = 'model.downloading';
              const progress = card.querySelector('.model-progress');
              progress.style.display = 'flex';

              try {
                await invoke('download_model', { modelId, url, destDir });
              } catch (e) {
                btn.disabled = false;
                btn.textContent = i18n.t('model.download');
                btn.dataset.i18n = 'model.download';
                progress.style.display = 'none';
                alert(i18n.t('model.download_error') + ': ' + e);
              }
            });
          });
          // Обработчики кнопок «Удалить» — удаляют файлы модели физически.
          // Конфиг при этом не трогаем (whisper_model_path и т.п. остаются),
          // но статус в карточке обновится на «Не найдена», и UI предложит скачать заново.
          container.querySelectorAll('.model-delete-btn').forEach(btn => {
            btn.addEventListener('click', async () => {
              const modelId = btn.dataset.id;
              const path = btn.dataset.path;
              const name = btn.dataset.name;
              const size = btn.dataset.size;
              if (!path) return;
              const confirmMsg = i18n.t('model.delete_confirm')
                .replace('{name}', name)
                .replace('{size}', size);
              if (!confirm(confirmMsg)) return;
              btn.disabled = true;
              try {
                await invoke('delete_model', { modelId, path });
                showToast(i18n.t('model.deleted'), 'success', 1500);
                // Backend очистил config-путь если он совпадал с удалённым → переподтянем
                // originalConfig и dropdown'ы (иначе fallback в getModelPathFromCard
                // вернёт старый путь и save восстановит). Полный loadSettings также
                // перерисовывает model-cards с пустым input для удалённой.
                await loadSettings();
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
        } catch (e) {
          container.innerHTML = `<div style="color:#565f89;font-size:0.8rem;">${i18n.t('model.load_error')}</div>`;
        }
      }

      // Активные загрузки моделей. Хранятся вне DOM, чтобы пережить re-render
      // при `loadSettings()` (например, после удаления другой модели). Без этого
      // прогресс-бар Large пропадал когда юзер удалял Tiny во время скачивания Large.
      // Ключ — model_id, значение — последний снимок прогресса.
      const activeDownloads = new Map();

      // Применить состояние прогресса к карточке (после re-render или нового события).
      function applyProgressToCard(modelId) {
        const card = document.querySelector(`.model-card[data-model-id="${modelId}"]`);
        if (!card) return;
        const prog = activeDownloads.get(modelId);
        if (!prog) return;
        const wrap = card.querySelector('.model-progress');
        const fill = card.querySelector('.model-progress-fill');
        const text = card.querySelector('.model-progress-text');
        if (wrap) wrap.style.display = 'flex';
        if (fill) fill.style.width = prog.percent + '%';
        if (text) {
          text.textContent = prog.total_files > 1
            ? `[${prog.file_idx + 1}/${prog.total_files}] ${prog.percent}%`
            : `${prog.percent}%`;
        }
        // На время загрузки прячем кнопку «Скачать» (если в свежем рендере она есть).
        const btn = card.querySelector('.model-download-btn');
        if (btn) {
          btn.disabled = true;
          btn.textContent = i18n.t('model.downloading');
        }
      }

      // Прогресс скачивания модели — обновляем глобальный state + DOM.
      listen('download://progress', (ev) => {
        const { model_id, percent, file_idx, total_files, file } = ev.payload;
        activeDownloads.set(model_id, { percent, file_idx, total_files, file });
        applyProgressToCard(model_id);
      });

      // Backend начал распаковку архива (Vosk качается одним .zip ~1.8 ГБ → 2.6 ГБ).
      // Без этого события UI бы видел застывший 100% на 30-90с — выглядит как зависание.
      // Меняем текст кнопки и прогресс-метку на «Распаковка…»; `loadSettings()` в
      // download://complete потом перерисует карточку начисто.
      listen('download://extracting', (ev) => {
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
      listen('download://complete', (ev) => {
        if (ev && ev.payload && ev.payload.model_id) {
          activeDownloads.delete(ev.payload.model_id);
        }
        loadSettings().catch(() => {});
      });

      // Собираем текущие значения из формы
      // Получить путь модели из карточки по типу транскрайбера. Если карточка не
      // отрендерилась или input пустой — НЕ возвращаем '', а откатываемся в
      // originalConfig (тот, который только что прислал backend). Иначе одно
      // несвоевременное Save обнулит whisper_model_path / model_path / qwen3asr_*
      // и engine упадёт с ошибкой "из :".
      function getModelPathFromCard(ttype) {
        const input = document.querySelector(`.model-path-input[data-type="${ttype}"]`);
        const fromCard = input ? input.value.trim() : '';
        if (fromCard) return fromCard;
        const fieldByType = {
          vosk: 'model_path',
          whisper: 'whisper_model_path',
          gigaam: 'gigaam_model_path',
          qwen3asr: 'qwen3asr_model_path',
        };
        const field = fieldByType[ttype];
        return (originalConfig && field && originalConfig[field]) || '';
      }

      // Текущее активное устройство ввода (имя из cpal). Хранится локально, чтоб
      // collectMicGainOverrides знал ключ, под которым обновлять mic_gain_per_device.
      let activeMicDevice = '';

      // Запрашивает у backend имя default-микрофона, обновляет UI ("Активный микрофон: ...")
      // и подставляет в ползунок gain текущий override (или global mic_gain как fallback).
      // Вызывается при загрузке Settings и по кнопке ↻ Обновить.
      async function refreshMicDevice(cfg) {
        try {
          activeMicDevice = await invoke('get_default_input_device_name');
        } catch (_) { activeMicDevice = ''; }
        const labelEl = document.getElementById('s-mic-active-device');
        if (labelEl) {
          labelEl.textContent = activeMicDevice || '—';
          labelEl.title = activeMicDevice || '';  // полное имя в tooltip если обрезалось
        }
        const gainEl = document.getElementById('s-mic-gain');
        if (!gainEl) return;
        const overrides = (cfg && cfg.mic_gain_per_device) || {};
        const fallback = (cfg && typeof cfg.mic_gain === 'number') ? cfg.mic_gain : 1.0;
        const current = activeMicDevice && overrides[activeMicDevice] != null
          ? overrides[activeMicDevice]
          : fallback;
        gainEl.value = current.toFixed(1);
      }

      // Берёт map из originalConfig и обновляет в нём gain для активного устройства
      // текущим значением ползунка. Возвращает обновлённую копию для save_config.
      // Важно: если новое значение совпадает с тем, что показывалось при загрузке
      // (override либо global fallback) — НЕ создаём override-запись. Иначе
      // change-detection видит diff даже когда пользователь ничего не менял
      // (или вернул значение обратно).
      function collectMicGainOverrides() {
        const base = (originalConfig && originalConfig.mic_gain_per_device) || {};
        const result = { ...base };
        if (!activeMicDevice) return result;
        const v = Math.max(0.5, Math.min(10, parseFloat(document.getElementById('s-mic-gain').value) || 1.0));
        const displayedAtLoad = base[activeMicDevice] != null
          ? base[activeMicDevice]
          : ((originalConfig && originalConfig.mic_gain) || 1.0);
        if (Math.abs(v - displayedAtLoad) < 0.01) {
          // Значение не изменилось от загрузки — оставляем map как есть.
          // Если override уже был — он сохраняется. Если не было — не создаём.
          return result;
        }
        // Реальное изменение — записываем override для активного устройства.
        result[activeMicDevice] = v;
        return result;
      }

      function getFormConfig() {
        // Dropdown даёт расщеплённое значение для whisper ('whisper-tiny' / 'whisper-large');
        // backend хранит просто transcriber="whisper" + whisper_model_path. Мапим обратно.
        const dropdownValue = document.getElementById('s-transcriber').dataset.value;
        let transcriber = dropdownValue;
        let whisperPathOverride = null;
        if (dropdownValue === 'whisper-tiny' || dropdownValue === 'whisper-large') {
          transcriber = 'whisper';
          const baseDir = document.getElementById('s-models-base-dir').value || '';
          const sep = baseDir.endsWith('/') ? '' : '/';
          whisperPathOverride = baseDir + sep + (dropdownValue === 'whisper-tiny'
            ? 'ggml-tiny.bin'
            : 'ggml-large-v3-turbo.bin');
        }
        return {
          transcriber,
          model_path: getModelPathFromCard('vosk'),
          whisper_model_path: whisperPathOverride || getModelPathFromCard('whisper'),
          gigaam_model_path: getModelPathFromCard('gigaam'),
          qwen3asr_model_path: getModelPathFromCard('qwen3asr'),
          models_base_dir: document.getElementById('s-models-base-dir').value,
          sample_rate: parseInt(document.getElementById('s-sample-rate').dataset.value),
          max_record_secs: parseInt(document.getElementById('s-max-record').value),
          hotkey: getHotkeyValue('hk-trigger'),
          hotkey_pause: getHotkeyValue('hk-pause'),
          auto_type: document.getElementById('s-auto-type').classList.contains('on'),
          debug: document.getElementById('s-debug').classList.contains('on'),
          vad_enabled: document.getElementById('s-vad-enabled').classList.contains('on'),
          vad_silence_secs: parseInt(document.getElementById('s-vad-silence').value) || 3,
          remove_fillers: document.getElementById('s-remove-fillers').classList.contains('on'),
          // mic_gain (глобальный fallback) и mic_gain_per_device (per-device override).
          // Глобальный mic_gain не редактируется через UI — это просто запоминание
          // последнего значения. Per-device map обновляется в refreshMicDevice/save.
          mic_gain: originalConfig && typeof originalConfig.mic_gain === 'number' ? originalConfig.mic_gain : 1.0,
          mic_gain_per_device: collectMicGainOverrides(),
          retention_hours: parseInt(document.getElementById('s-retention').dataset.value) || 0,
          autostart: document.getElementById('s-autostart').classList.contains('on'),
          start_minimized: document.getElementById('s-start-minimized').classList.contains('on'),
          show_widget: document.getElementById('s-show-widget').classList.contains('on'),
          widget_position: document.getElementById('s-widget-position').dataset.value || 'bottom-center',
          show_tray: document.getElementById('s-show-tray').classList.contains('on'),
          preload_models: normalizePreload([
            ...(document.getElementById('s-preload-vosk').classList.contains('on') ? ['vosk'] : []),
            ...(document.getElementById('s-preload-whisper').classList.contains('on') ? ['whisper'] : []),
            ...(document.getElementById('s-preload-gigaam').classList.contains('on') ? ['gigaam'] : []),
            ...(document.getElementById('s-preload-qwen3asr').classList.contains('on') ? ['qwen3asr'] : []),
          ], normalizeTranscriber(document.getElementById('s-transcriber').dataset.value)),
          language: document.getElementById('s-language').dataset.value || i18n.getLanguage(),
          history_filter_secs: parseInt(document.getElementById('h-period').dataset.value) || 0,
        };
      }

      // Маппинг полей конфига → DOM элементов для подсветки
      const fieldMap = {
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
        preload_models:    { el: 's-preload-vosk',   type: 'row' },
      };

      // Проверка изменений + подсветка изменённых полей
      function checkChanges() {
        if (!originalConfig) return;
        const current = getFormConfig();
        let anyChanged = false;

        for (const [key, info] of Object.entries(fieldMap)) {
          const changed = JSON.stringify(current[key]) !== JSON.stringify(originalConfig[key]);
          const el = document.getElementById(info.el);
          const container = info.type === 'group' ? el.closest('.setting-group') : el.closest('.setting-row');
          if (container) container.classList.toggle('changed', changed);
          if (changed) anyChanged = true;
        }

        saveBtn.disabled = !anyChanged;
        cancelBtn.disabled = !anyChanged;
        saveNotice.classList.remove('visible');
      }

      // Кастомные dropdown-ы с portal (dropdown переносится в body при открытии)
      function initCustomSelects() {
        document.querySelectorAll('.custom-select').forEach(sel => {
          if (sel._initialized) return;
          sel._initialized = true;

          const trigger = sel.querySelector('.custom-select-trigger');
          const optionsEl = sel.querySelector('.custom-select-options');
          const options = sel.querySelectorAll('.custom-select-option');

          const initVal = sel.dataset.value;
          const initOpt = sel.querySelector(`[data-value="${initVal}"]`);
          if (initOpt) {
            trigger.textContent = initOpt.textContent;
            initOpt.classList.add('selected');
          }

          function openDropdown() {
            closeAllDropdowns();
            // Portal: переносим options в body с fixed позицией
            const rect = trigger.getBoundingClientRect();
            document.body.appendChild(optionsEl);
            optionsEl.classList.add('portal');
            optionsEl.style.top = rect.bottom + 'px';
            optionsEl.style.left = rect.left + 'px';
            optionsEl.style.width = rect.width + 'px';
            sel.classList.add('open');
          }

          function closeDropdown() {
            optionsEl.classList.remove('portal');
            optionsEl.style.display = '';
            optionsEl.style.top = '';
            optionsEl.style.left = '';
            optionsEl.style.width = '';
            if (optionsEl.parentNode !== sel) {
              sel.appendChild(optionsEl);
            }
            sel.classList.remove('open');
          }

          trigger.addEventListener('click', (e) => {
            e.stopPropagation();
            if (sel.classList.contains('open')) closeDropdown();
            else openDropdown();
          });

          options.forEach(opt => {
            opt.addEventListener('click', (e) => {
              e.stopPropagation();
              // Опция помечена disabled (движок не включён в текущую сборку,
              // см. applyEngineAvailability). Игнорируем клик, dropdown остаётся открытым,
              // чтобы пользователь увидел доступные пункты.
              if (opt.classList.contains('option--disabled')) return;
              sel.dataset.value = opt.dataset.value;
              trigger.textContent = opt.textContent;
              options.forEach(o => o.classList.remove('selected'));
              opt.classList.add('selected');
              closeDropdown();
              // Уведомляем об изменении
              sel.dispatchEvent(new Event('change'));
            });
          });

          sel._close = closeDropdown;
        });
      }

      function closeAllDropdowns() {
        document.querySelectorAll('.custom-select').forEach(s => { if (s._close) s._close(); });
      }

      document.addEventListener('click', closeAllDropdowns);
      initCustomSelects();

      // Навешиваем обработчики изменений
      ['s-models-base-dir', 's-max-record', 's-vad-silence', 's-mic-gain']
        .forEach(id => document.getElementById(id).addEventListener('input', checkChanges));

      // Кнопка ↻ Обновить — пере-запрашивает активный микрофон у backend и
      // подставляет соответствующий override в ползунок. Полезно когда пользователь
      // переключает мик в системе (Bluetooth headset, USB, встроенный) без перезапуска UI.
      document.getElementById('s-mic-refresh').addEventListener('click', async () => {
        await refreshMicDevice(originalConfig);
        checkChanges();  // ползунок мог поменять значение → обновить dirty-state
      });
      // Кастомные стрелки для number-input
      document.querySelectorAll('.number-input-wrap').forEach(wrap => {
        const input = wrap.querySelector('input[type="number"]');
        wrap.querySelector('.arrow-up').addEventListener('click', () => { input.stepUp(); input.dispatchEvent(new Event('input')); });
        wrap.querySelector('.arrow-down').addEventListener('click', () => { input.stepDown(); input.dispatchEvent(new Event('input')); });
      });
      // Кнопка выбора базовой директории моделей
      document.getElementById('pick-base-dir').addEventListener('click', async () => {
        const currentPath = document.getElementById('s-models-base-dir').value || '';
        const selected = await window.__TAURI__.dialog.open({
          directory: true,
          title: i18n.t('model.pick_base_title'),
          defaultPath: currentPath,
        });
        if (selected) {
          document.getElementById('s-models-base-dir').value = selected;
          checkChanges();
        }
      });
      ['s-transcriber', 's-sample-rate', 's-retention']
        .forEach(id => document.getElementById(id).addEventListener('change', checkChanges));
      // При смене модели по умолчанию — обновить блокировки предзагрузки + предупредить
      // если выбрана тяжёлая модель и у CPU нет AVX2 (Whisper Large без AVX2 крайне медленный).
      document.getElementById('s-transcriber').addEventListener('change', () => {
        const v = document.getElementById('s-transcriber').dataset.value;
        updatePreloadLocks(v);
        const heavyOnNoAvx2 =
          (v === 'whisper-large' || v === 'qwen3asr') &&
          cpuFeatures && cpuFeatures.avx2 === false;
        if (heavyOnNoAvx2) {
          showToast(i18n.t('settings.slow_model_warning'), 'warning', 7000);
        }
      });

      // Смена языка — применяется сразу и сохраняется автоматически
      document.getElementById('s-language').addEventListener('change', async () => {
        const lang = document.getElementById('s-language').dataset.value;
        i18n.setLanguage(lang);
        try {
          await invoke('set_language', { lang });
          showToast(i18n.t('toast.saved'), 'success', 1500);
        } catch (e) {
          showToast(`${i18n.t('toast.save_error')}: ${e}`, 'error', 3000);
        }
      });

      // === Композер горячих клавиш ===
      // Модификаторы выбираются кнопками, основная клавиша — через рекордер.
      // Super на Wayland не доходит до WebView, поэтому кнопка вместо keydown.

      // Маппинг JS event.code → Tauri формат
      function mapKeyCode(event) {
        const code = event.code;
        if (code.startsWith('Key')) return code.slice(3);
        if (code.startsWith('Digit')) return code.slice(5);
        if (/^F\d+$/.test(code)) return code;
        const map = {
          Space: 'Space', Enter: 'Return', Tab: 'Tab',
          Backspace: 'Backspace', Delete: 'Delete', Insert: 'Insert',
          ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right',
          Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown',
          Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']',
          Semicolon: ';', Quote: "'", Backquote: '`', Backslash: '\\',
          Comma: ',', Period: '.', Slash: '/',
        };
        return map[code] || event.key;
      }

      // Собрать комбинацию из состояния композера
      function composeHotkey(composer) {
        const mods = [];
        composer.querySelectorAll('.hotkey-mod.active').forEach(btn => {
          mods.push(btn.dataset.mod);
        });
        const keyInput = composer.querySelector('.hotkey-key-input');
        const key = keyInput.dataset.key || '';
        if (!key) return '';
        return [...mods, key].join('+');
      }

      // Обновить превью
      function updateHotkeyPreview(composer) {
        const combo = composeHotkey(composer);
        const preview = composer.querySelector('.hotkey-preview');
        preview.dataset.value = combo;
        preview.textContent = combo || '';
      }

      // Получить/установить значение
      function getHotkeyValue(id) {
        return composeHotkey(document.getElementById(id));
      }

      function setHotkeyValue(id, value) {
        const composer = document.getElementById(id);
        // Сбрасываем всё
        composer.querySelectorAll('.hotkey-mod').forEach(btn => btn.classList.remove('active'));
        const keyInput = composer.querySelector('.hotkey-key-input');
        keyInput.value = '';
        keyInput.dataset.key = '';

        if (!value) {
          updateHotkeyPreview(composer);
          return;
        }

        // Парсим "Super+Shift+G" → mods + key
        const parts = value.split('+');
        const modNames = ['Super', 'Control', 'Alt', 'Shift'];
        for (const part of parts) {
          if (modNames.includes(part)) {
            const btn = composer.querySelector(`.hotkey-mod[data-mod="${part}"]`);
            if (btn) btn.classList.add('active');
          } else {
            keyInput.value = part;
            keyInput.dataset.key = part;
          }
        }
        updateHotkeyPreview(composer);
      }

      // Инициализация композера
      function initHotkeyComposer(composerId) {
        const composer = document.getElementById(composerId);

        // Toggle модификаторов
        composer.querySelectorAll('.hotkey-mod').forEach(btn => {
          btn.addEventListener('click', () => {
            btn.classList.toggle('active');
            updateHotkeyPreview(composer);
            checkChanges();
          });
        });

        // Рекордер основной клавиши
        const keyInput = composer.querySelector('.hotkey-key-input');
        const recordBtn = composer.querySelector('.hotkey-record-btn');
        const clearBtn = composer.querySelector('.hotkey-clear-btn');

        recordBtn.addEventListener('click', () => {
          keyInput.classList.add('recording');
          keyInput.value = 'Нажмите клавишу...';
          keyInput.focus();

          const handler = (e) => {
            e.preventDefault();
            e.stopPropagation();

            // Пропускаем модификаторы — они управляются кнопками
            if (['Control', 'Alt', 'Shift', 'Meta', 'Super', 'OS'].includes(e.key)) return;

            // Escape — отмена
            if (e.key === 'Escape') {
              keyInput.classList.remove('recording');
              keyInput.value = keyInput.dataset.key || '';
              document.removeEventListener('keydown', handler, true);
              return;
            }

            const mapped = mapKeyCode(e);
            keyInput.dataset.key = mapped;
            keyInput.value = mapped;
            keyInput.classList.remove('recording');
            document.removeEventListener('keydown', handler, true);
            updateHotkeyPreview(composer);
            checkChanges();
          };

          document.addEventListener('keydown', handler, true);
        });

        // Очистка
        clearBtn.addEventListener('click', () => {
          setHotkeyValue(composerId, '');
          checkChanges();
        });
      }

      initHotkeyComposer('hk-trigger');
      initHotkeyComposer('hk-pause');

      // Проверяем Wayland и показываем предупреждение
      invoke('is_wayland').then(wayland => {
        if (wayland) {
          document.getElementById('wayland-warning').style.display = 'block';
          // Хинт под picker'ом — mutter может проигнорировать выбор позиции
          const hint = document.getElementById('s-widget-position-hint');
          if (hint) hint.hidden = false;
        }
      });

      // GNOME-Wayland: показываем toggle GNOME-расширения для точного позиционирования.
      // На X11 / KDE / sway / Cinnamon — расширение не применимо, ряд скрыт.
      Promise.all([invoke('is_wayland'), invoke('is_gnome'), invoke('widget_extension_status')])
        .then(([wayland, gnome, status]) => {
          if (!(wayland && gnome && status.available)) return;
          document.getElementById('s-widget-ext-row').hidden = false;
          document.getElementById('s-widget-ext-hint').hidden = false;
          // Текущее состояние toggle = enabled в gsettings (status.enabled).
          // Если установлено но не enabled — toggle off (пользователь выключил вручную).
          document.getElementById('s-widget-ext').classList.toggle('on', status.enabled);
        }).catch(() => { /* gsettings/non-GNOME — игнорируем */ });

      // Click-handler на toggle расширения: install + enable + предложить logout, либо disable.
      document.getElementById('s-widget-ext').addEventListener('click', async () => {
        const toggle = document.getElementById('s-widget-ext');
        const wantOn = !toggle.classList.contains('on');
        // Оптимистично переключаем визуально; откатим если бэкенд вернёт ошибку.
        toggle.classList.toggle('on', wantOn);
        try {
          if (wantOn) {
            // backend возвращает true если уже было включено — тогда не предлагаем relogin
            const wasAlreadyEnabled = await invoke('install_widget_extension');
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
      document.querySelectorAll('#s-widget-position .position-picker-dot').forEach(dot => {
        dot.addEventListener('click', () => {
          setWidgetPositionPicker(dot.dataset.pos);
          checkChanges();
        });
      });

      // Переключение табов настроек
      document.querySelectorAll('.settings-tab').forEach(tab => {
        tab.addEventListener('click', () => {
          document.querySelectorAll('.settings-tab').forEach(t => t.classList.remove('active'));
          document.querySelectorAll('.settings-tab-content').forEach(c => c.classList.remove('active'));
          tab.classList.add('active');
          document.getElementById('tab-' + tab.dataset.tab).classList.add('active');
          closeAllDropdowns();
        });
      });

      // Toggle-переключатели
      ['s-auto-type', 's-debug', 's-vad-enabled', 's-remove-fillers', 's-autostart', 's-start-minimized', 's-show-tray', 's-show-widget', 's-preload-vosk', 's-preload-whisper', 's-preload-gigaam', 's-preload-qwen3asr'].forEach(id => {
        document.getElementById(id).addEventListener('click', () => {
          document.getElementById(id).classList.toggle('on');
          checkChanges();
        });
      });

      // Отмена изменений — восстанавливаем оригинальные значения.
      // Async чтобы дождаться refreshMicDevice (он делает invoke get_default_input_device_name).
      // Без await checkChanges() видел старое значение ползунка и не сбрасывал dirty-state
      // с первого клика — приходилось жать дважды.
      cancelBtn.addEventListener('click', async () => {
        if (!originalConfig) return;
        setCustomSelect('s-transcriber', originalConfig.transcriber);
        document.getElementById('s-models-base-dir').value = originalConfig.models_base_dir || '';
        renderModelCards();
        setCustomSelect('s-sample-rate', String(originalConfig.sample_rate));
        document.getElementById('s-max-record').value = originalConfig.max_record_secs;
        setHotkeyValue('hk-trigger', originalConfig.hotkey);
        setHotkeyValue('hk-pause', originalConfig.hotkey_pause || '');
        document.getElementById('s-auto-type').classList.toggle('on', originalConfig.auto_type);
        document.getElementById('s-debug').classList.toggle('on', originalConfig.debug);
        document.getElementById('s-vad-enabled').classList.toggle('on', originalConfig.vad_enabled !== false);
        document.getElementById('s-vad-silence').value = originalConfig.vad_silence_secs || 7;
        document.getElementById('s-remove-fillers').classList.toggle('on', originalConfig.remove_fillers !== false);
        // Ждём пока refreshMicDevice реально обновит ползунок ДО checkChanges.
        await refreshMicDevice(originalConfig);
        setCustomSelect('s-retention', String(originalConfig.retention_hours || 0));
        document.getElementById('s-autostart').classList.toggle('on', originalConfig.autostart || false);
        document.getElementById('s-start-minimized').classList.toggle('on', originalConfig.start_minimized || false);
        document.getElementById('s-show-widget').classList.toggle('on', originalConfig.show_widget !== false);
        setWidgetPositionPicker(originalConfig.widget_position || 'bottom-center');
        document.getElementById('s-show-tray').classList.toggle('on', originalConfig.show_tray !== false);
        const preloadOrig = originalConfig.preload_models || [];
        document.getElementById('s-preload-vosk').classList.toggle('on', preloadOrig.includes('vosk'));
        document.getElementById('s-preload-whisper').classList.toggle('on', preloadOrig.includes('whisper'));
        document.getElementById('s-preload-gigaam').classList.toggle('on', preloadOrig.includes('gigaam'));
        document.getElementById('s-preload-qwen3asr').classList.toggle('on', preloadOrig.includes('qwen3asr'));
        checkChanges();
      });

      // Сохранение
      saveBtn.addEventListener('click', async () => {
        const cfg = getFormConfig();
        await invoke('save_config', { config: cfg });

        // На Wayland — проверяем конфликты и регистрируем хоткеи через gsettings
        try {
          const wayland = await invoke('is_wayland');
          if (wayland) {
            // Проверяем конфликты
            const conflicts = [];
            if (cfg.hotkey) {
              const c = await invoke('check_hotkey_conflict', { hotkey: cfg.hotkey });
              if (c) conflicts.push(i18n.t('hotkey.trigger_conflict', { hotkey: cfg.hotkey, holder: c }));
            }
            if (cfg.hotkey_pause) {
              const c = await invoke('check_hotkey_conflict', { hotkey: cfg.hotkey_pause });
              if (c) conflicts.push(i18n.t('hotkey.pause_conflict', { hotkey: cfg.hotkey_pause, holder: c }));
            }
            if (conflicts.length > 0) {
              const msg = i18n.t('hotkey.conflicts_prompt', { list: conflicts.join('\n') });
              if (!confirm(msg)) return;
            }

            await invoke('register_gnome_hotkeys', {
              hotkeyTrigger: cfg.hotkey,
              hotkeyPause: cfg.hotkey_pause,
            });
          }
        } catch (e) {
          console.warn('Не удалось зарегистрировать GNOME хоткеи:', e);
        }

        originalConfig = JSON.parse(JSON.stringify(cfg));
        checkChanges();
        saveNotice.classList.add('visible');
        updateModelBadge();
      });

      /* ──────── История ──────── */
      const historyPage = document.getElementById('history-page');
      const historyList = document.getElementById('history-list');
      const historyEmpty = document.getElementById('history-empty');
      const histPagination = document.getElementById('history-pagination');
      const histPrev = document.getElementById('h-prev');
      const histNext = document.getElementById('h-next');
      const histPageInfo = document.getElementById('h-page-info');
      const HIST_PER_PAGE = 10;
      let histPage = 0;

      // Навигация в меню → история
      document.getElementById('menu-history').addEventListener('click', () => {
        showPage('history');
        histPage = 0;
        loadHistory();
      });

      document.getElementById('menu-about').addEventListener('click', () => {
        showPage('about');
      });

      // Обновить showPage для history
      const origShowPage = showPage;
      showPage = function(page) {
        contentEl.style.display = page === 'main' ? '' : 'none';
        menuPage.classList.toggle('visible', page === 'menu');
        settingsPage.classList.toggle('visible', page === 'settings');
        historyPage.classList.toggle('visible', page === 'history');
        document.getElementById('about-page').classList.toggle('visible', page === 'about');
        closeAllDropdowns();
        menuBtn.classList.toggle('back', page !== 'main');
        currentPage = page;
        if (page === 'main') updateModelBadge();
      };

      // Обновить кнопку "Назад" для истории
      const origMenuClick = menuBtn.onclick;
      menuBtn.removeEventListener('click', menuBtn._handler);
      menuBtn._handler = () => {
        if (currentPage === 'main') showPage('menu');
        else if (currentPage === 'settings' || currentPage === 'history' || currentPage === 'about') {
          showPage('menu');
          saveNotice.classList.remove('visible');
        } else showPage('main');
      };
      menuBtn.addEventListener('click', menuBtn._handler);

      // Фильтр периода (кастомный select — инициализируем)
      const hPeriod = document.getElementById('h-period');
      initCustomSelects();
      hPeriod.addEventListener('change', async () => {
        histPage = 0;
        loadHistory();
        try {
          await invoke('set_history_filter', { secs: parseInt(hPeriod.dataset.value) || 0 });
          showToast(i18n.t('toast.saved'), 'success', 1500);
        } catch (e) {
          showToast(`${i18n.t('toast.save_error')}: ${e}`, 'error', 3000);
        }
      });

      async function loadHistory() {
        const sinceSecs = parseInt(hPeriod.dataset.value);
        const result = await invoke('get_history', { sinceSecs, limit: HIST_PER_PAGE, offset: histPage * HIST_PER_PAGE });
        renderHistory(result.entries, result.total);
      }

      // Восстановить язык интерфейса и период фильтра истории из конфига
      async function restoreUiState() {
        try {
          const cfg = await invoke('load_config');
          if (cfg.language) {
            i18n.setLanguage(cfg.language);
          } else {
            i18n.applyI18n();
          }
          const savedSecs = cfg.history_filter_secs;
          if (savedSecs !== undefined && savedSecs !== null) {
            const opt = hPeriod.querySelector(`[data-value="${savedSecs}"]`);
            if (opt) setCustomSelect('h-period', String(savedSecs));
          }
        } catch (_) {
          i18n.applyI18n();
        }
      }
      restoreUiState();

      function fmtDate(ts) {
        const d = new Date(ts * 1000);
        const locale = i18n.getLanguage() === 'en' ? 'en-US' : 'ru-RU';
        return d.toLocaleString(locale, { day: '2-digit', month: '2-digit', year: 'numeric', hour: '2-digit', minute: '2-digit' });
      }

      function fmtDuration(secs) {
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
        { type: 'gigaam', label: 'GigaAM' },
        { type: 'qwen3asr', label: 'Qwen3-ASR' },
      ];

      function renderHistory(entries, total) {
        historyList.innerHTML = '';
        if (entries.length === 0) {
          historyEmpty.style.display = 'block';
          histPagination.classList.remove('visible');
          return;
        }
        historyEmpty.style.display = 'none';

        for (const entry of entries) {
          const r = entry.recording;
          const trans = entry.transcriptions;
          // Самая ранняя транскрибация — последняя в массиве (DESC порядок)
          const firstTrans = trans.length > 0 ? trans[trans.length - 1] : null;
          const defaultModel = firstTrans ? firstTrans.model_name : ALL_MODELS[0].label;

          // Собираем модели: существующие транскрибации + одна "нераспознано" для другого типа
          const modelOptions = [];
          const seenTypes = new Set();
          const originalModel = firstTrans ? firstTrans.model_name : null;
          for (const t of trans) {
            modelOptions.push({ label: t.model_name, text: t.text, type: t.transcriber_type, hasText: true, isOriginal: t.model_name === originalModel });
            seenTypes.add(t.transcriber_type);
          }
          // Добавляем вариант "нераспознано" для типов, которых ещё нет
          for (const m of MODEL_TYPES) {
            if (!seenTypes.has(m.type)) {
              modelOptions.push({ label: m.label + ' ' + i18n.t('history.not_recognized'), text: null, type: m.type, hasText: false });
            }
          }

          const div = document.createElement('div');
          div.className = 'hist-entry';

          // Первая строка: дата, длительность, кнопки
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
          div.appendChild(metaDiv);

          // Вторая строка: кастомный dropdown с моделями
          const selectWrap = document.createElement('div');
          selectWrap.className = 'custom-select';
          selectWrap.dataset.value = defaultModel;
          selectWrap.style.margin = '0.4rem 0';
          selectWrap.innerHTML = `
            <div class="custom-select-trigger${defaultModel === originalModel ? ' original' : ''}">${defaultModel}${defaultModel === originalModel ? ' ★' : ''}</div>
            <div class="custom-select-options">
              ${modelOptions.map(m =>
                `<div class="custom-select-option${m.label === defaultModel ? ' selected' : ''}" data-value="${m.label}" data-type="${m.type}" data-has-text="${m.hasText}">${m.label}${m.isOriginal ? ' ★' : ''}</div>`
              ).join('')}
            </div>
          `;
          div.appendChild(selectWrap);

          // Третья строка: текст или кнопка "Распознать"
          const contentDiv = document.createElement('div');
          const defaultOpt = modelOptions.find(m => m.label === defaultModel);
          if (defaultOpt && defaultOpt.hasText) {
            contentDiv.className = 'hist-text';
            contentDiv.textContent = defaultOpt.text;
          } else if (entry.audio_exists) {
            contentDiv.innerHTML = `<button class="hist-retranscribe-btn" data-rid="${r.id}" data-type="${defaultOpt ? defaultOpt.type : 'whisper'}" data-i18n="history.recognize_btn">${i18n.t('history.recognize_btn')}</button>`;
          } else {
            contentDiv.innerHTML = `<span style="color:#565f89;font-size:0.8rem;font-style:italic;" data-i18n="history.audio_deleted">${i18n.t('history.audio_deleted')}</span>`;
          }
          div.appendChild(contentDiv);

          // Инициализация кастомного select
          initCustomSelects();

          // Переключение модели
          selectWrap.addEventListener('change', () => {
            const selectedLabel = selectWrap.dataset.value;
            const opt = modelOptions.find(m => m.label === selectedLabel);
            const trigger = selectWrap.querySelector('.custom-select-trigger');
            trigger.classList.toggle('original', opt && opt.isOriginal);
            trigger.textContent = selectedLabel + (opt && opt.isOriginal ? ' ★' : '');
            contentDiv.innerHTML = '';
            contentDiv.className = '';
            if (opt && opt.hasText) {
              contentDiv.className = 'hist-text';
              contentDiv.textContent = opt.text;
            } else if (entry.audio_exists) {
              contentDiv.innerHTML = `<button class="hist-retranscribe-btn" data-rid="${r.id}" data-type="${opt ? opt.type : 'whisper'}" data-i18n="history.recognize_btn">${i18n.t('history.recognize_btn')}</button>`;
              contentDiv.querySelector('.hist-retranscribe-btn').addEventListener('click', doRetranscribe);
            } else {
              contentDiv.innerHTML = `<span style="color:#565f89;font-size:0.8rem;font-style:italic;" data-i18n="history.audio_deleted">${i18n.t('history.audio_deleted')}</span>`;
            }
          });

          // Кнопка распознать (если сразу видна)
          const retBtn = contentDiv.querySelector('.hist-retranscribe-btn');
          if (retBtn) retBtn.addEventListener('click', doRetranscribe);

          async function doRetranscribe(e) {
            const btn = e.target;
            const rid = parseInt(btn.dataset.rid);
            const ttype = btn.dataset.type;
            btn.textContent = i18n.t('history.recognizing');
            btn.dataset.i18n = 'history.recognizing';
            btn.disabled = true;
            try {
              await invoke('retranscribe', { recordingId: rid, transcriberType: ttype });
              loadHistory();
            } catch (err) {
              alert(i18n.t('toast.error') + ': ' + err);
              btn.textContent = i18n.t('history.recognize_btn');
              btn.dataset.i18n = 'history.recognize_btn';
              btn.disabled = false;
            }
          }

          // Копировать
          actionsDiv.querySelector('.hist-btn.copy').addEventListener('click', async () => {
            const textEl = div.querySelector('.hist-text');
            if (textEl) await navigator.clipboard.writeText(textEl.textContent);
          });

          // Удалить
          actionsDiv.querySelector('.hist-btn.delete').addEventListener('click', async () => {
            await invoke('delete_history_entry', { recordingId: r.id });
            loadHistory();
          });

          // Воспроизвести запись
          const playBtn = actionsDiv.querySelector('.hist-btn.play');
          if (playBtn) {
            playBtn.addEventListener('click', () => playAudio(parseInt(playBtn.dataset.rid), playBtn));
          }

          historyList.appendChild(div);
        }

        // Пагинация
        const totalPages = Math.ceil(total / HIST_PER_PAGE);
        if (totalPages > 1) {
          histPagination.classList.add('visible');
          histPrev.disabled = histPage === 0;
          histNext.disabled = histPage >= totalPages - 1;
          histPageInfo.textContent = i18n.t('history.page', { current: histPage + 1, total: totalPages });
        } else {
          histPagination.classList.remove('visible');
        }
      }

      // Воспроизведение аудио через Web Audio API
      const SVG_PLAY = '<svg viewBox="0 0 24 24"><polygon points="5,3 19,12 5,21"/></svg>';
      const SVG_STOP = '<svg viewBox="0 0 24 24"><rect x="4" y="4" width="16" height="16" rx="2"/></svg>';
      const SVG_PAUSE = '<svg viewBox="0 0 24 24"><rect x="5" y="4" width="4" height="16"/><rect x="15" y="4" width="4" height="16"/></svg>';
      const SVG_RESUME = '<svg viewBox="0 0 24 24"><polygon points="5,3 19,12 5,21"/></svg>';

      let currentPlayer = null; // { source, ctx, playBtn, pauseBtn, paused }

      function stopCurrentPlayer() {
        if (!currentPlayer) return;
        currentPlayer.source.stop();
        currentPlayer.ctx.close();
        currentPlayer.playBtn.innerHTML = SVG_PLAY;
        currentPlayer.playBtn.classList.remove('playing');
        if (currentPlayer.pauseBtn) currentPlayer.pauseBtn.remove();
        currentPlayer = null;
      }

      async function playAudio(recordingId, playBtn) {
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
          const result = await invoke('get_audio_data', { recordingId });
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
          playBtn.parentNode.insertBefore(pauseBtn, playBtn);

          let paused = false;
          pauseBtn.addEventListener('click', () => {
            if (!currentPlayer) return;
            if (paused) {
              audioCtx.resume();
              pauseBtn.innerHTML = SVG_PAUSE;
              pauseBtn.title = i18n.t('mic.pause');
              pauseBtn.dataset.i18nTitle = 'mic.pause';
              paused = false;
            } else {
              audioCtx.suspend();
              pauseBtn.innerHTML = SVG_RESUME;
              pauseBtn.title = i18n.t('mic.resume');
              pauseBtn.dataset.i18nTitle = 'mic.resume';
              paused = true;
            }
          });

          currentPlayer = { source, ctx: audioCtx, playBtn, pauseBtn, paused };

          source.onended = () => {
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

      histPrev.addEventListener('click', () => { if (histPage > 0) { histPage--; loadHistory(); historyPage.scrollTop = 0; } });
      histNext.addEventListener('click', () => { histPage++; loadHistory(); historyPage.scrollTop = 0; });

      // Очистить историю
      // Кастомный модальный диалог
      function showConfirm(text, confirmLabel, cancelLabel) {
        return new Promise(resolve => {
          const overlay = document.getElementById('modal-overlay');
          const confirmEl = document.getElementById('modal-confirm');
          const cancelEl = document.getElementById('modal-cancel');
          const origConfirmText = confirmEl.textContent;
          const origCancelText = cancelEl.textContent;
          document.getElementById('modal-text').textContent = text;
          if (confirmLabel) confirmEl.textContent = confirmLabel;
          if (cancelLabel) cancelEl.textContent = cancelLabel;
          overlay.classList.add('visible');
          const onConfirm = () => { cleanup(); resolve(true); };
          const onCancel = () => { cleanup(); resolve(false); };
          const cleanup = () => {
            overlay.classList.remove('visible');
            confirmEl.removeEventListener('click', onConfirm);
            cancelEl.removeEventListener('click', onCancel);
            // Восстанавливаем оригинальные тексты, чтобы не сломать другие вызовы.
            confirmEl.textContent = origConfirmText;
            cancelEl.textContent = origCancelText;
          };
          confirmEl.addEventListener('click', onConfirm);
          cancelEl.addEventListener('click', onCancel);
        });
      }

      // Экспорт истории — dropdown
      const exportMenu = document.getElementById('export-menu');
      document.getElementById('h-export-btn').addEventListener('click', (e) => {
        e.stopPropagation();
        exportMenu.classList.toggle('visible');
      });
      document.addEventListener('click', () => exportMenu.classList.remove('visible'));
      exportMenu.addEventListener('click', (e) => e.stopPropagation());

      async function exportHistory(format) {
        exportMenu.classList.remove('visible');
        try {
          const path = await invoke('export_history', { format });
          showToast(i18n.t('toast.file_saved'), 'success', 5000);
        } catch (e) {
          showToast(`${i18n.t('toast.error')}: ${e}`, 'error', 3000);
        }
      }
      document.querySelectorAll('.export-dropdown-item').forEach(item => {
        item.addEventListener('click', () => exportHistory(item.dataset.format));
      });

      document.getElementById('h-clear-btn').addEventListener('click', async () => {
        if (await showConfirm(i18n.t('history.confirm_clear'))) {
          await invoke('clear_history');
          loadHistory();
        }
      });
