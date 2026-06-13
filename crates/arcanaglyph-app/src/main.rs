// crates/arcanaglyph-app/src/main.rs

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod models;
mod setup;
mod tray;
mod updater;

use arcanaglyph_core::CoreConfig;
use arcanaglyph_core::config::TranscriberType;
use commands::EngineState;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
use tauri_plugin_global_shortcut::ShortcutState;

fn main() {
    // Раннее: --grant-portal subcommand. Должен сработать ДО Tauri-инициализации
    // (нам не нужны окна / трей / engine — только portal warmup).
    if std::env::args().any(|a| a == "--grant-portal") {
        setup::run_grant_portal_and_exit();
    }

    // Инициализируем логирование. Дефолт — `info,whisper_rs=warn` (тихий whisper.cpp);
    // через `RUST_LOG` можно перебить (например `RUST_LOG=info,whisper_rs=trace` для
    // отладки whisper-инференса — увидим внутренние ggml/encoder сообщения).
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,whisper_rs=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Выбираем libonnxruntime.so для GigaAM/Qwen3-ASR. Делаем сразу после init трейсинга,
    // чтобы все последующие сообщения о выборе ORT попали в лог; и сильно ДО первого
    // обращения к `ort` (первое касание — в transcriber.rs при создании engine после
    // скачивания модели, спустя секунды).
    setup::setup_ort_dylib_path();
    // Принудительно ставим WM_CLASS=arcanaglyph (для группировки в GNOME Dash).
    setup::setup_program_name();

    let mut config = CoreConfig::load().unwrap_or_else(|e| {
        tracing::warn!("Не удалось загрузить конфиг: {}, используем дефолтные настройки", e);
        CoreConfig::default()
    });

    // Возможные причины авто-fallback'а на старте (показываются единым toast'ом в UI):
    //   1. В SQLite сохранён движок, не включённый в текущую сборку
    //      (например, ранее был Vosk, а сейчас собрано без feature `vosk`).
    //   2. Активный движок ONNX-based (GigaAM/Qwen3-ASR), а CPU без AVX.
    //      Эмпирически проверено: pre-built ONNX Runtime от Microsoft (тот, что качает
    //      `ort` крейт через `download-binaries`) на CPU без AVX крашит SIGILL ещё
    //      до первого `println!`. Поэтому, если у пользователя выбран ONNX-движок и
    //      нет AVX — мягко переключаемся на не-ONNX (Whisper или Vosk) и сохраняем
    //      выбор в БД, чтобы UI показал реальное состояние.
    let mut engine_fallback: Option<(String, String)> = None;

    // Случай 1: движок не включён в текущую сборку.
    // ВАЖНО: НЕ сохраняем fallback в БД — пользовательский выбор (например, GigaAM)
    // остаётся в конфиге как первичный. Если пользователь пересоберёт с другой
    // feature-set'ом или установит более производительный CPU — выбор GigaAM сразу
    // станет активным без необходимости восстанавливать настройку. Toast в UI
    // объяснит, какой именно движок реально использовался в этой сессии.
    if !config.transcriber.is_compiled_in() {
        let original = config.transcriber.as_str().to_string();
        let new_engine = TranscriberType::compiled_engines()
            .into_iter()
            .next()
            .unwrap_or_default();
        config.transcriber = new_engine;
        let fallback = config.transcriber.as_str().to_string();
        tracing::warn!(
            "Движок '{}' не включён в эту сборку — используется '{}' (runtime-fallback, БД не меняется)",
            original,
            fallback
        );
        engine_fallback = Some((original, fallback));
    }

    // Случай 2: ONNX-based движок на CPU без AVX. На не-x86_64 (aarch64 и т.д.)
    // считаем, что AVX-проблем нет — там используются другие SIMD-наборы.
    #[cfg(target_arch = "x86_64")]
    let avx_ok = std::is_x86_feature_detected!("avx");
    #[cfg(not(target_arch = "x86_64"))]
    let avx_ok = true;
    // ORT-фича `download-binaries` тянет Microsoft pre-built ORT — требует AVX.
    // `load-dynamic` (через `gigaam-system-ort`) использует локальную libonnxruntime.so
    // (см. `setup_ort_dylib_path()` выше). На наших .deb-сборках выбирается no-AVX-вариант,
    // поэтому AVX не нужен.
    // qwen3asr использует тот же ORT-крейт что и gigaam (после унификации feature'ов),
    // поэтому условие AVX-требования совпадает.
    let ort_needs_avx = cfg!(feature = "gigaam") && !cfg!(feature = "gigaam-system-ort");
    let needs_avx = match config.transcriber {
        TranscriberType::GigaAm | TranscriberType::Qwen3Asr => ort_needs_avx,
        _ => false,
    };
    if !avx_ok && needs_avx {
        let original = config.transcriber.as_str().to_string();
        // Ищем первый не-ONNX движок среди скомпилированных (Whisper, потом Vosk).
        let alt = TranscriberType::compiled_engines()
            .into_iter()
            .find(|t| !matches!(t, TranscriberType::GigaAm | TranscriberType::Qwen3Asr));
        if let Some(new_engine) = alt {
            config.transcriber = new_engine;
            let fallback = config.transcriber.as_str().to_string();
            // НЕ сохраняем в БД — пользовательский выбор (GigaAM) остаётся первичным.
            tracing::warn!(
                "CPU без AVX: '{}' требует ONNX Runtime с AVX — runtime-переключение на '{}' (БД не меняется)",
                original,
                fallback
            );
            engine_fallback = Some((original, fallback));
        } else {
            // Все скомпилированные движки требуют AVX (только gigaam/qwen3asr).
            // Engine всё равно создастся и упадёт SIGILL'ом — это редкий кейс
            // явной пользовательской ошибки при сборке.
            tracing::error!("CPU без AVX и нет не-ONNX движков в сборке — engine может крашить");
        }
    }

    let hotkey = config.hotkey.clone();
    let hotkey_pause = config.hotkey_pause.clone();

    // Строки хоткеев для сравнения в handler
    let trigger_hk = Arc::new(hotkey.clone());
    let pause_hk = Arc::new(hotkey_pause.clone());

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Второй экземпляр — показываем окно первого
            tray::show_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler({
                    let trigger_hk = Arc::clone(&trigger_hk);
                    let pause_hk = Arc::clone(&pause_hk);
                    move |app, shortcut, event| {
                        if event.state() == ShortcutState::Pressed
                            && let Some(engine_state) = app.try_state::<EngineState>()
                            && let Some(engine) = engine_state.get()
                        {
                            let sc_str = format!("{shortcut}");
                            if !pause_hk.is_empty() && sc_str == *pause_hk.as_ref() {
                                tracing::info!(source = "tauri-shortcut", "Горячая клавиша паузы: {}", sc_str);
                                engine.pause();
                            } else if sc_str == *trigger_hk.as_ref() {
                                tracing::info!(source = "tauri-shortcut", "Горячая клавиша триггера: {}", sc_str);
                                engine.trigger();
                            }
                        }
                    }
                })
                .build(),
        )
        .setup(move |app| setup::run_setup(app, hotkey, hotkey_pause, config, engine_fallback))
        .invoke_handler(tauri::generate_handler![
            commands::trigger,
            commands::pause,
            commands::cancel_transcription,
            commands::active_supports_cancel,
            commands::get_audio_level,
            commands::is_recording,
            commands::is_paused,
            commands::is_model_loaded,
            commands::get_loaded_models,
            commands::get_compiled_engines,
            commands::get_cpu_features,
            commands::get_default_input_device_name,
            models::registry::get_models,
            models::registry::download_model,
            models::registry::delete_model,
            commands::is_wayland,
            commands::is_gnome,
            commands::check_portal_grant_needed,
            commands::grant_portal_now,
            commands::get_app_version,
            commands::check_updates_now,
            commands::dismiss_update,
            commands::open_release_notes,
            commands::apply_update,
            commands::clear_update_applying,
            commands::get_update_applying,
            commands::restart_app,
            commands::widget_extension_status,
            commands::install_widget_extension,
            commands::disable_widget_extension,
            commands::request_logout,
            commands::check_hotkey_conflict,
            commands::register_gnome_hotkeys,
            commands::hide_window,
            commands::load_config,
            commands::save_config,
            commands::set_history_filter,
            commands::set_language,
            commands::get_history,
            commands::delete_history_entry,
            commands::clear_history,
            commands::export_history,
            commands::retranscribe,
            commands::get_audio_data,
        ])
        .on_window_event(|window, event| {
            // Перехватываем закрытие окна — скрываем вместо закрытия
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                // Обновляем флаг видимости только для главного окна
                if window.label() == "main"
                    && let Some(vis) = window.app_handle().try_state::<Arc<AtomicBool>>()
                {
                    vis.store(false, Ordering::Relaxed);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Ошибка запуска Tauri");
}
