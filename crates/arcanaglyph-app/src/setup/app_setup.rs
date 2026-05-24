// crates/arcanaglyph-app/src/setup/app_setup.rs
//
// `run_setup` — тело Tauri-замыкания `.setup(...)`, вытащенное в обычную функцию
// с явными параметрами. Создаёт виджет, регистрирует tauri-plugin хоткеи,
// авторегистрирует GNOME custom-keybindings (Wayland/X11), спавнит engine в фоне
// после ensure_active_model, поднимает auto-cleanup истории, update checker и
// UDP listener.

use crate::commands::EngineState;
use crate::commands::hotkeys::register_gnome_hotkeys;
use crate::models;
use crate::setup::bootstrap::install_wayland_scripts;
use crate::setup::events::{run_engine_event_loop, spawn_udp_listener, spawn_update_checker};
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

    install_wayland_scripts();

    // Автоочистка старых записей при старте + периодически
    if let Ok(cfg) = CoreConfig::load()
        && cfg.retention_hours > 0
        && let (Some(db_path), Some(cache)) = (CoreConfig::history_db_path(), CoreConfig::audio_cache_dir())
        && let Ok(db) = arcanaglyph_core::history::HistoryDB::new(&db_path, cache)
    {
        let _ = db.cleanup_old_recordings(cfg.retention_hours);
    }

    // Периодическая очистка: интервал = retention_hours
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

    // Проверяем start_minimized до инициализации движка
    let start_minimized = CoreConfig::load().map(|c| c.start_minimized).unwrap_or(false);

    let window_visible = Arc::new(AtomicBool::new(!start_minimized));
    app.manage(window_visible.clone());

    // Если запуск в свёрнутом виде — скрываем окно сразу
    if start_minimized && let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }

    // Engine создаётся в фоне — окно показывается сразу (если не minimized)
    let engine_state: EngineState = Arc::new(OnceLock::new());
    app.manage(engine_state.clone());

    // Временная HistoryDB до загрузки engine
    let db_path = CoreConfig::history_db_path().ok_or_else(|| "Не удалось определить путь БД".to_string())?;
    let audio_cache = CoreConfig::audio_cache_dir().ok_or_else(|| "Не удалось определить путь кэша".to_string())?;
    let history_db = Arc::new(HistoryDB::new(&db_path, audio_cache).map_err(|e| e.to_string())?);
    app.manage(history_db.clone());

    // === Update checker ===
    // 0. Восстановление applying-режима после реального restart.
    //    Если applying_version == APP_VERSION — установка прошла
    //    успешно, чистим метку. Иначе — эмитим applying, чтобы UI
    //    показал баннер «Установка идёт... / Перезапустить».
    //    Делаем это ДО cached_pending_update, чтобы applying имел
    //    приоритет над available для той же версии.
    {
        let state = updater::read_state(&history_db);
        if let Some(v) = state.applying_version.clone() {
            if v == updater::APP_VERSION {
                let _ = updater::clear_applying(&history_db);
            } else {
                let _ = app.handle().emit("update://applying", v);
            }
        }
    }
    // 1. Если в state уже знаем про более свежий релиз — эмитим
    //    `update://available` сразу, чтобы UI получил баннер до
    //    первой фоновой проверки (которая через 60 секунд).
    //    `cached_pending_update` сам фильтрует applying_version,
    //    так что available + applying для одной версии не конфликтуют.
    if let Some(info) = updater::cached_pending_update(&history_db) {
        let _ = app.handle().emit("update://available", info);
    }
    // 2. Фоновый чекер с exponential backoff.
    spawn_update_checker(app.handle().clone(), Arc::clone(&history_db));

    // Скачивание модели и загрузка engine в фоне.
    // Выполняем СТРОГО последовательно: сначала убеждаемся, что модель активного
    // движка на диске (качаем при первом запуске), затем создаём engine. Это убирает
    // ERROR-логи при первом запуске («failed to open <model>») — engine видит файл.
    let app_handle_load = app.handle().clone();
    let engine_state_load = engine_state.clone();
    let engine_fallback_evt = engine_fallback.clone();
    let active_transcriber = config.transcriber.as_str().to_string();
    // Копию widget_position берём заранее — после spawn'а ниже config moves в engine
    let widget_position = config.widget_position.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = models::download::ensure_active_model(&active_transcriber, &app_handle_load).await {
            tracing::error!("Не удалось подготовить модель для '{}': {}", active_transcriber, e);
            // Парсим строку ошибки в ApiError — формат events.rs::EngineEvent::Error
            // ожидает frontend; UI получит правильный kind (DiskSpace / Network / ModelLoad)
            // и подходящий hint.
            let api_err = ApiError::from_message(&format!("Не удалось скачать модель: {}", e));
            let _ = app_handle_load.emit("engine://error", &api_err);
            return;
        }

        let result = tokio::task::spawn_blocking(move || ArcanaEngine::new(config, window_visible)).await;

        match result {
            Ok(Ok(engine)) => {
                // Подписываемся на события ПЕРЕД set, пока есть ownership
                let rx = engine.subscribe();
                let _ = engine_state_load.set(engine);
                tracing::info!("Engine готов к работе");
                let _ = app_handle_load.emit("engine://model-loaded", serde_json::json!({}));

                // Если при старте сработал auto-fallback на дефолтный движок — сообщаем UI,
                // чтобы тот показал toast «движок X недоступен, используется Y».
                if let Some((original, fallback)) = engine_fallback_evt {
                    let _ = app_handle_load.emit(
                        "engine://fallback",
                        serde_json::json!({ "original": original, "fallback": fallback }),
                    );
                }

                // Предзагрузка дополнительных моделей в фоне
                if engine_state_load.get().is_some() {
                    let preload_list: Vec<_> = {
                        let cfg = arcanaglyph_core::CoreConfig::load().ok();
                        cfg.map(|c| c.preload_models).unwrap_or_default()
                    };
                    for t_type in preload_list {
                        let app_h = app_handle_load.clone();
                        let es = engine_state_load.clone();
                        tokio::task::spawn_blocking(move || {
                            if let Some(e) = es.get() {
                                match e.preload_model(&t_type) {
                                    Ok(name) => {
                                        tracing::info!("Модель '{}' предзагружена", name);
                                        let _ = app_h
                                            .emit("engine://model-preloaded", serde_json::json!({ "model": name }));
                                    }
                                    Err(err) => tracing::warn!("Не удалось предзагрузить модель: {}", err),
                                }
                            }
                        });
                    }
                }

                // Event loop: пробрасываем события engine → фронтенд
                tokio::spawn(run_engine_event_loop(
                    app_handle_load.clone(),
                    engine_state_load.clone(),
                    rx,
                ));
            }
            Ok(Err(e)) => {
                tracing::error!("Ошибка создания engine: {}", e);
                // `e` тут — `ArcanaError`, поэтому конвертим напрямую через from_arcana
                // (точнее чем from_message — не теряем kind на парсинге строки).
                let api_err = ApiError::from_arcana(&e);
                let _ = app_handle_load.emit("engine://error", &api_err);
            }
            Err(e) => {
                tracing::error!("Ошибка загрузки: {:?}", e);
            }
        }
    });

    // Создаём иконку в системном трее
    if let Err(e) = tray::create_tray(app) {
        tracing::error!("Не удалось создать иконку в трее: {}", e);
    }

    // Скрываем иконку трея если выключена в настройках
    if !CoreConfig::load().map(|c| c.show_tray).unwrap_or(true) {
        tray::set_tray_visible(app.handle(), false);
    }

    // Создаём виджет записи программно (для точного контроля размера)
    {
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
                &widget_position,
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

    // Авторегистрация горячих клавиш в GNOME (Wayland И X11) при первом запуске.
    // Запускаем на любом GNOME-сеансе:
    // - на Wayland tauri-plugin-global-shortcut вообще не работает (нет X11);
    // - на X11+GNOME он часто не получает event'ы (mutter их перехватывает).
    // Нативные GNOME custom-keybindings → ag-trigger → UDP — единственный
    // надёжный путь для GNOME. Для не-GNOME DE (KDE/i3/sway) этот блок просто
    // тихо отвалится с ошибкой gsettings — там нужно настраивать вручную.
    #[cfg(target_os = "linux")]
    if !hotkey.is_empty() {
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
                Ok(out) => {
                    let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    val.is_empty() || val == "''" || val.contains("No such")
                }
                Err(_) => true,
            }
        };
        let needs_register = probe("arcanaglyph-trigger")
            || probe("arcanaglyph-trigger-cyr")
            || probe("arcanaglyph-pause")
            || probe("arcanaglyph-pause-cyr");
        if needs_register {
            tracing::info!("Регистрирую глобальные горячие клавиши в GNOME...");
            if let Err(e) = register_gnome_hotkeys(hotkey.clone(), hotkey_pause.clone()) {
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
            let mut layout = String::new();
            let mut variant = String::new();
            for line in String::from_utf8_lossy(&query.stdout).lines() {
                if let Some(rest) = line.strip_prefix("layout:") {
                    layout = rest.trim().to_string();
                } else if let Some(rest) = line.strip_prefix("variant:") {
                    variant = rest.trim().to_string();
                }
            }
            if !layout.is_empty() {
                let mut args = vec!["-layout".to_string(), layout.clone()];
                if !variant.is_empty() {
                    args.push("-variant".to_string());
                    args.push(variant.clone());
                }
                let _ = std::process::Command::new("setxkbmap").args(&args).output();
                tracing::info!(
                    "XKB пнут (layout={}, variant={}) — mutter пере-grab'ит cyr keysym'ы",
                    layout,
                    variant
                );
            }
        }
    }

    // UDP-триггер для Wayland (внешний скрипт ag-trigger → UDP :9002)
    spawn_udp_listener(engine_state.clone());

    Ok(())
}
