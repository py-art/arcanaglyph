// crates/arcanaglyph-app/src/setup/app_setup.rs
//
// `run_setup` — тело Tauri-замыкания `.setup(...)`, вытащенное в обычную функцию
// с явными параметрами. Создаёт виджет, регистрирует tauri-plugin хоткеи,
// авторегистрирует GNOME custom-keybindings (Wayland/X11), спавнит engine в фоне
// после ensure_active_model, поднимает auto-cleanup истории, update checker и
// IPC trigger listener (Unix-сокет, только Linux).
//
// Тело разбито на связные step-функции: `run_setup` — линейный оркестратор,
// каждый шаг изолирован (ниже по файлу). Чистая логика probe/XKB — в
// `commands::hotkeys` (тестируется без tauri).

use crate::commands::EngineState;
use crate::models;
use crate::setup::bootstrap::cleanup_legacy_scripts;
#[cfg(target_os = "linux")]
use crate::setup::events::spawn_trigger_listener;
use crate::setup::events::{run_engine_event_loop, spawn_update_checker};
use crate::tray;
use crate::updater;
use arcanaglyph_core::error::ApiError;
use arcanaglyph_core::history::HistoryDB;
use arcanaglyph_core::{ArcanaEngine, CoreConfig};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

pub fn run_setup(
    app: &mut tauri::App,
    hotkey: String,
    hotkey_pause: String,
    config: CoreConfig,
    engine_fallback: Option<(String, String)>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Создаём директорию моделей если не существует. Сами модели качаем
    // позже через `ensure_active_model` — единым generic-механизмом для всех движков.
    if let Some(models_dir) = CoreConfig::models_dir() {
        let _ = std::fs::create_dir_all(&models_dir);
    }

    cleanup_legacy_scripts();

    // Синхронизируем autostart с конфигом при старте — на всех платформах.
    // Иначе при обновлении с уже включённой галочкой (autostart=true в БД)
    // фактическая регистрация автозапуска делается ТОЛЬКО на UI-save, а
    // пользователь её не пересохраняет (тумблер и так «вкл») → на Windows ключ
    // реестра HKCU\...\Run / на macOS LaunchAgent plist так и не создаётся.
    // На Linux вдобавок перезаписывает устаревший `Exec=` (dev-путь от
    // `make run` или после переустановки). `set_autostart` идемпотентна и
    // определена для всех таргетов (no-op для прочих ОС).
    crate::setup::bootstrap::set_autostart(config.autostart);

    run_startup_history_cleanup();
    spawn_retention_cleanup();

    let window_visible = init_window_visibility(app);

    // Engine создаётся в фоне — окно показывается сразу (если не minimized)
    let engine_state: EngineState = Arc::new(OnceLock::new());
    app.manage(engine_state.clone());

    let history_db = init_history_db(app)?;
    init_update_checker(app, &history_db);

    // Копию widget_position берём заранее — ниже config moves в engine.
    let widget_position = config.widget_position.clone();
    spawn_engine_loader(
        app.handle().clone(),
        engine_state.clone(),
        config,
        window_visible,
        engine_fallback,
    );

    init_tray(app);
    build_recording_widget(app, &widget_position);
    register_app_hotkeys(app, &hotkey, &hotkey_pause);

    // macOS гейтит вставку (Accessibility) и глобальный хоткей (Input Monitoring)
    // отдельными грантами — логируем их статус при старте, чтобы denied было
    // видно из лог-файла сразу (тест на macOS идёт через друга, цикл медленный).
    #[cfg(target_os = "macos")]
    crate::setup::macos_permissions::log_macos_permission_status();

    #[cfg(target_os = "linux")]
    ensure_gnome_hotkeys(&hotkey, &hotkey_pause);

    // Проактивный warmup XDG RemoteDesktop при старте (Wayland, нет токена):
    // portal-popup всплывёт при запуске — спокойный момент, — а не лениво при
    // первом Ctrl+Ё (диалог посреди записи). После первого согласия токен
    // переиспользуется и диалога больше нет.
    #[cfg(target_os = "linux")]
    spawn_portal_warmup();

    // IPC-триггер для Linux (GNOME-хоткей → `arcanaglyph --trigger` → Unix-сокет).
    // На Win/macOS хоткей ловится in-process плагином global-shortcut — слушатель
    // не нужен.
    #[cfg(target_os = "linux")]
    spawn_trigger_listener(engine_state.clone());

    Ok(())
}

/// Разовая очистка старых записей при старте (если включён retention).
fn run_startup_history_cleanup() {
    if let Ok(cfg) = CoreConfig::load()
        && cfg.retention_hours > 0
        && let (Some(db_path), Some(cache)) = (CoreConfig::history_db_path(), CoreConfig::audio_cache_dir())
        && let Ok(db) = arcanaglyph_core::history::HistoryDB::new(&db_path, cache)
    {
        let _ = db.cleanup_old_recordings(cfg.retention_hours);
    }
}

/// Проактивный warmup XDG RemoteDesktop при старте. Если на Wayland ещё нет
/// сохранённого restore_token — запрашиваем разрешение в фоне сейчас (popup в
/// момент запуска), чтобы не ловить его лениво при первом Ctrl+Ё посреди записи.
/// Best-effort: ошибку только логируем (UI-баннер портала остаётся fallback'ом).
#[cfg(target_os = "linux")]
fn spawn_portal_warmup() {
    if !arcanaglyph_core::input::needs_portal_grant() {
        return;
    }
    tauri::async_runtime::spawn(async {
        if let Err(e) = arcanaglyph_core::input::warmup_remote_desktop().await {
            tracing::warn!("Проактивный warmup RemoteDesktop не удался: {e}");
        }
    });
}

/// Периодическая очистка истории: интервал = `retention_hours` (читается каждый цикл).
fn spawn_retention_cleanup() {
    tauri::async_runtime::spawn(async {
        loop {
            let hours = CoreConfig::load().map(|c| c.retention_hours).unwrap_or(0);
            if hours == 0 {
                // Хранить вечно — спим час и проверяем снова (вдруг настройку поменяли)
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                continue;
            }
            // Спим retention_hours, потом чистим
            tokio::time::sleep(std::time::Duration::from_secs(hours * 3600)).await;

            if let (Some(db_path), Some(cache)) = (CoreConfig::history_db_path(), CoreConfig::audio_cache_dir())
                && let Ok(db) = arcanaglyph_core::history::HistoryDB::new(&db_path, cache)
            {
                let _ = db.cleanup_old_recordings(hours);
            }
        }
    });
}

/// Создаёт флаг видимости окна (по `start_minimized`), регистрирует его в Tauri
/// и прячет окно если запуск свёрнутый. Возвращает Arc для передачи в engine.
fn init_window_visibility(app: &mut tauri::App) -> Arc<AtomicBool> {
    // Проверяем start_minimized до инициализации движка
    let start_minimized = CoreConfig::load().map(|c| c.start_minimized).unwrap_or(false);

    let window_visible = Arc::new(AtomicBool::new(!start_minimized));
    app.manage(window_visible.clone());

    // Окно создаётся скрытым (`"visible": false` в tauri.conf) — показываем его
    // ТОЛЬКО если старт не свёрнутый. Так нет гонки «окно мелькнуло и спряталось»:
    // при start_minimized оно вообще не появляется. Прежний подход (visible:true +
    // hide() здесь) был ненадёжен — компоновщик успевал показать окно до hide().
    if !start_minimized && let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }

    window_visible
}

/// Создаёт временную `HistoryDB` (до загрузки engine) и регистрирует её в Tauri.
fn init_history_db(app: &mut tauri::App) -> Result<Arc<HistoryDB>, Box<dyn std::error::Error>> {
    let db_path = CoreConfig::history_db_path().ok_or_else(|| "Не удалось определить путь БД".to_string())?;
    let audio_cache = CoreConfig::audio_cache_dir().ok_or_else(|| "Не удалось определить путь кэша".to_string())?;
    let history_db = Arc::new(HistoryDB::new(&db_path, audio_cache).map_err(|e| e.to_string())?);
    app.manage(history_db.clone());
    Ok(history_db)
}

/// Восстанавливает applying-режим после restart, эмитит cached-available и
/// запускает фоновый update-checker.
fn init_update_checker(app: &tauri::App, history_db: &Arc<HistoryDB>) {
    // 0. Сброс устаревшей applying-метки на старте приложения.
    //    На СВЕЖЕМ старте метка всегда устаревшая, что бы в ней ни стояло:
    //      * applying == APP_VERSION → установка прошла успешно (версия догнала);
    //      * applying != APP_VERSION → установка НЕ завершилась (пользователь
    //        закрыл терминал install.sh до конца / отмена / падение — версия
    //        не догнала).
    //    В обоих случаях метку снимаем. Раньше для несовпавшей версии здесь
    //    повторно эмитился `update://applying`, и при прерванной установке
    //    приложение НАВСЕГДА залипало в баннере «Установка идёт… / Перезапустить»
    //    (applying глушит баннер «Доступно», кнопка «Перезапустить» ждёт флаг
    //    готовности, которого после обрыва нет) — перезапуск приложения не лечил.
    //    Сняв метку, отдаём приоритет `cached_pending_update` ниже: при
    //    незавершённой установке снова покажется баннер «Доступно», и пользователь
    //    сможет повторить обновление.
    if updater::read_state(history_db).applying_version.is_some() {
        let _ = updater::clear_applying(history_db);
    }
    // 1. Если в state уже знаем про более свежий релиз — эмитим
    //    `update://available` сразу, чтобы UI получил баннер до
    //    первой фоновой проверки (которая через 60 секунд).
    //    `cached_pending_update` сам фильтрует applying_version,
    //    так что available + applying для одной версии не конфликтуют.
    if let Some(info) = updater::cached_pending_update(history_db) {
        let _ = app.handle().emit("update://available", info);
    }
    // 2. Фоновый чекер с exponential backoff.
    spawn_update_checker(app.handle().clone(), Arc::clone(history_db));
}

/// Скачивает модель активного движка и создаёт engine в фоне (строго
/// последовательно: сначала модель на диске, затем `ArcanaEngine::new`), затем
/// подписывает event-loop и предзагружает дополнительные модели.
fn spawn_engine_loader(
    app_handle: tauri::AppHandle,
    engine_state: EngineState,
    config: CoreConfig,
    window_visible: Arc<AtomicBool>,
    engine_fallback: Option<(String, String)>,
) {
    let active_transcriber = config.transcriber.as_str().to_string();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = models::download::ensure_active_model(&active_transcriber, &app_handle).await {
            tracing::error!("Не удалось подготовить модель для '{}': {}", active_transcriber, e);
            // Парсим строку ошибки в ApiError — формат events.rs::EngineEvent::Error
            // ожидает frontend; UI получит правильный kind (DiskSpace / Network / ModelLoad)
            // и подходящий hint.
            let api_err = ApiError::from_message(&format!("Не удалось скачать модель: {}", e));
            let _ = app_handle.emit("engine://error", &api_err);
            return;
        }

        let result = tokio::task::spawn_blocking(move || ArcanaEngine::new(config, window_visible)).await;

        match result {
            Ok(Ok(engine)) => {
                // Подписываемся на события ПЕРЕД set, пока есть ownership
                let rx = engine.subscribe();
                let _ = engine_state.set(engine);
                tracing::info!("Engine готов к работе");
                let _ = app_handle.emit("engine://model-loaded", serde_json::json!({}));

                // Если при старте сработал auto-fallback на дефолтный движок — сообщаем UI,
                // чтобы тот показал toast «движок X недоступен, используется Y».
                if let Some((original, fallback)) = engine_fallback {
                    let _ = app_handle.emit(
                        "engine://fallback",
                        serde_json::json!({ "original": original, "fallback": fallback }),
                    );
                }

                // Предзагрузка дополнительных моделей в фоне
                spawn_model_preload(&app_handle, &engine_state);

                // Event loop: пробрасываем события engine → фронтенд
                tokio::spawn(run_engine_event_loop(app_handle.clone(), engine_state.clone(), rx));
            }
            Ok(Err(e)) => {
                tracing::error!("Ошибка создания engine: {}", e);
                // `e` тут — `ArcanaError`, поэтому конвертим напрямую через from_arcana
                // (точнее чем from_message — не теряем kind на парсинге строки).
                let api_err = ApiError::from_arcana(&e);
                let _ = app_handle.emit("engine://error", &api_err);
            }
            Err(e) => {
                tracing::error!("Ошибка загрузки: {:?}", e);
            }
        }
    });
}

/// Предзагружает в фоне дополнительные модели из `config.preload_models` — каждую в
/// своём `spawn_blocking`, чтобы не блокировать. На успех эмитит `engine://model-
/// preloaded`. Вынесено из `spawn_engine_loader` ради снятия глубокой вложенности
/// (был nesting 6). Async/Tauri-bound — проверяется live-verify, не юнитом.
fn spawn_model_preload(app_handle: &tauri::AppHandle, engine_state: &EngineState) {
    if engine_state.get().is_none() {
        return;
    }
    let preload_list: Vec<_> = arcanaglyph_core::CoreConfig::load()
        .ok()
        .map(|c| c.preload_models)
        .unwrap_or_default();
    for t_type in preload_list {
        let app_h = app_handle.clone();
        let es = engine_state.clone();
        tokio::task::spawn_blocking(move || {
            let Some(e) = es.get() else {
                return;
            };
            match e.preload_model(&t_type) {
                Ok(name) => {
                    tracing::info!("Модель '{}' предзагружена", name);
                    let _ = app_h.emit("engine://model-preloaded", serde_json::json!({ "model": name }));
                }
                Err(err) => tracing::warn!("Не удалось предзагрузить модель: {}", err),
            }
        });
    }
}

/// Создаёт иконку в системном трее (и скрывает её, если выключена в настройках).
fn init_tray(app: &tauri::App) {
    if let Err(e) = tray::create_tray(app) {
        tracing::error!("Не удалось создать иконку в трее: {}", e);
    }

    // Скрываем иконку трея если выключена в настройках
    if !CoreConfig::load().map(|c| c.show_tray).unwrap_or(true) {
        tray::set_tray_visible(app.handle(), false);
    }
}

/// Создаёт виджет записи программно (для точного контроля размера) и позиционирует
/// его по `widget_position`.
fn build_recording_widget(app: &tauri::App, widget_position: &str) {
    let widget_width = 220.0;
    let widget_height = 40.0;
    let mut builder = tauri::WebviewWindowBuilder::new(app, "widget", tauri::WebviewUrl::App("widget.html".into()))
        // Title используется GNOME-расширением arcanaglyph-widget@arfi.tech
        // для идентификации именно этого окна (wm_class общий с главным).
        .title("ArcanaGlyph Recording Widget")
        .inner_size(widget_width, widget_height)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .visible(false)
        .skip_taskbar(true);

    // Позиционируем виджет по выбору пользователя (config.widget_position).
    // На Wayland mutter может проигнорировать приложенческое позиционирование
    // (security-model `xdg_toplevel`) — это ожидаемо, в UI показывается хинт.
    if let Some(monitor) = app.primary_monitor().ok().flatten() {
        let screen = monitor.size();
        let scale = monitor.scale_factor();
        let (x, y) = arcanaglyph_core::config::widget_position_xy(
            widget_position,
            screen.width as f64 / scale,
            screen.height as f64 / scale,
            widget_width,
            widget_height,
        );
        builder = builder.position(x, y);
    }

    if let Err(e) = builder.build() {
        tracing::error!("Не удалось создать виджет записи: {}", e);
    }
}

/// Регистрирует глобальные горячие клавиши через tauri-plugin-global-shortcut
/// (trigger + опциональная пауза).
fn register_app_hotkeys(app: &tauri::App, hotkey: &str, hotkey_pause: &str) {
    // Регистрируем глобальные горячие клавиши
    match hotkey.parse::<tauri_plugin_global_shortcut::Shortcut>() {
        Ok(shortcut) => {
            if let Err(e) = app.global_shortcut().register(shortcut) {
                tracing::error!("Не удалось зарегистрировать горячую клавишу '{}': {}", hotkey, e);
            } else {
                tracing::info!("Горячая клавиша '{}' зарегистрирована", hotkey);
            }
        }
        Err(e) => {
            tracing::error!("Невалидная горячая клавиша '{}': {}", hotkey, e);
        }
    }

    // Регистрируем горячую клавишу паузы (если задана)
    if !hotkey_pause.is_empty() {
        match hotkey_pause.parse::<tauri_plugin_global_shortcut::Shortcut>() {
            Ok(shortcut) => {
                if let Err(e) = app.global_shortcut().register(shortcut) {
                    tracing::error!("Не удалось зарегистрировать клавишу паузы '{}': {}", hotkey_pause, e);
                } else {
                    tracing::info!("Горячая клавиша паузы '{}' зарегистрирована", hotkey_pause);
                }
            }
            Err(e) => {
                tracing::error!("Невалидная клавиша паузы '{}': {}", hotkey_pause, e);
            }
        }
    }
}

// Авторегистрация горячих клавиш в GNOME (Wayland И X11) при первом запуске.
// Запускаем на любом GNOME-сеансе:
// - на Wayland tauri-plugin-global-shortcut вообще не работает (нет X11);
// - на X11+GNOME он часто не получает event'ы (mutter их перехватывает).
// Нативные GNOME custom-keybindings → ag-trigger → UDP — единственный
// надёжный путь для GNOME. Для не-GNOME DE (KDE/i3/sway) этот блок просто
// тихо отвалится с ошибкой gsettings — там нужно настраивать вручную.
#[cfg(target_os = "linux")]
fn ensure_gnome_hotkeys(hotkey: &str, hotkey_pause: &str) {
    use crate::commands::hotkeys::{
        binding_is_empty, build_setxkbmap_args, command_is_legacy, parse_setxkbmap_query, register_gnome_hotkeys,
    };

    if hotkey.is_empty() {
        return;
    }

    // Проверяем, зарегистрированы ли уже наши хоткеи. Перерегистрируем если
    // отсутствует ЛЮБОЙ из четырёх slots — особенно cyr-варианты, которые
    // могли не быть созданы старой версией кода без mapping для grave.
    let probe = |slot: &str| -> bool {
        let check = std::process::Command::new("gsettings")
            .args([
                "get",
                &format!(
                    "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/{}/",
                    slot
                ),
                "binding",
            ])
            .output();
        match check {
            Ok(out) => binding_is_empty(String::from_utf8_lossy(&out.stdout).trim()),
            Err(_) => true,
        }
    };
    // Миграция со старого механизма: до перехода на Unix-сокет command хоткея
    // указывал на bash-скрипт ag-trigger (UDP :9002). Теперь это `<exe> --trigger`.
    // Если в slot осталась старая форма — принудительно перерегистрируем, иначе
    // обновившийся пользователь продолжил бы дёргать удалённый скрипт.
    let cmd_outdated = |slot: &str| -> bool {
        let check = std::process::Command::new("gsettings")
            .args([
                "get",
                &format!(
                    "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/{}/",
                    slot
                ),
                "command",
            ])
            .output();
        match check {
            Ok(out) => command_is_legacy(&String::from_utf8_lossy(&out.stdout)),
            Err(_) => false,
        }
    };
    let needs_register = probe("arcanaglyph-trigger")
        || probe("arcanaglyph-trigger-cyr")
        || probe("arcanaglyph-pause")
        || probe("arcanaglyph-pause-cyr")
        || cmd_outdated("arcanaglyph-trigger")
        || cmd_outdated("arcanaglyph-pause");
    if needs_register {
        tracing::info!("Регистрирую глобальные горячие клавиши в GNOME...");
        if let Err(e) = register_gnome_hotkeys(hotkey.to_string(), hotkey_pause.to_string()) {
            tracing::warn!("Не удалось зарегистрировать GNOME-хоткеи (не GNOME?): {}", e);
        }
    }

    // Пинок XKB на X11+GNOME: заставляем mutter пере-grab'ить keysym'ы.
    // Без этого на свежезагруженной системе кириллические keybindings
    // (`<Control>Cyrillic_io` для Ё) НЕ срабатывают пока пользователь
    // не переключит раскладку хотя бы раз вручную (Super+Space).
    // setxkbmap с теми же параметрами триггерит XKB-reload и mutter
    // пересоздаёт grabs, включая Cyrillic_io.
    let is_x11 = std::env::var("XDG_SESSION_TYPE").map(|v| v == "x11").unwrap_or(false);
    if is_x11 && let Ok(query) = std::process::Command::new("setxkbmap").arg("-query").output() {
        let (layout, variant) = parse_setxkbmap_query(&String::from_utf8_lossy(&query.stdout));
        if !layout.is_empty() {
            let args = build_setxkbmap_args(&layout, &variant);
            let _ = std::process::Command::new("setxkbmap").args(&args).output();
            tracing::info!(
                "XKB пнут (layout={}, variant={}) — mutter пере-grab'ит cyr keysym'ы",
                layout,
                variant
            );
        }
    }
}
