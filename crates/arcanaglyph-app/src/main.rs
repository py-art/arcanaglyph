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

/// Настраивает подписчика трейсинга с двумя слоями: stdout и файл (ротация по
/// дням, non-blocking). Возвращает `WorkerGuard` файлового аппендера — его нужно
/// держать живым до конца процесса, иначе буфер не успеет дописаться при выходе.
/// Если каталог логов недоступен (нет HOME/XDG, ошибка mkdir) — логируем только
/// в stdout и возвращаем `None`.
fn init_logging() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    // Дефолт — `info,whisper_rs=warn` (тихий whisper.cpp); через `RUST_LOG` можно
    // перебить (например `RUST_LOG=info,whisper_rs=trace` для отладки инференса).
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,whisper_rs=warn"));
    let stdout_layer = tracing_subscriber::fmt::layer();

    match CoreConfig::logs_dir() {
        Some(dir) if std::fs::create_dir_all(&dir).is_ok() => {
            // Ротация по дням + ретенция последних 7 файлов
            // (`arcanaglyph.log.YYYY-MM-DD`): старые удаляются автоматически.
            // Без лимита (`rolling::daily`) логи копились бесконечно, в т.ч. со
            // старых версий — особенно заметно на Windows, где файл живёт долго.
            let appender = tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("arcanaglyph.log")
                .max_log_files(7)
                .build(&dir);
            match appender {
                Ok(file_appender) => {
                    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
                    // with_ansi(false): без escape-кодов цвета — файл читаемый в Блокноте.
                    let file_layer = tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(non_blocking);
                    tracing_subscriber::registry()
                        .with(filter)
                        .with(stdout_layer)
                        .with(file_layer)
                        .init();
                    Some(guard)
                }
                // Не смогли построить файловый appender — остаёмся на stdout-only.
                Err(_) => {
                    tracing_subscriber::registry().with(filter).with(stdout_layer).init();
                    None
                }
            }
        }
        _ => {
            tracing_subscriber::registry().with(filter).with(stdout_layer).init();
            None
        }
    }
}

/// Ставит panic-hook, который пишет панику через `tracing::error!` (→ в файл).
/// Без этого на Windows (нет консоли) паника убивала бы процесс молча, без следа.
/// Дефолтный hook тоже вызываем — чтобы в dev паника по-прежнему печаталась в stderr.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<неизвестно>".to_string());
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<не-строковый payload паники>".to_string());
        tracing::error!(target: "panic", location = %location, "PANIC: {}", msg);
        default_hook(info);
    }));
}

/// Пишет в лог стартовую диагностику: версия, ОС, архитектура, наличие AVX и путь
/// к файлу логов. Эти строки — первое, что мы попросим у пользователя при разборе
/// проблем на Windows (AVX особенно: его отсутствие = SIGILL в ORT-движках).
fn log_startup_diagnostics() {
    #[cfg(target_arch = "x86_64")]
    let avx = std::is_x86_feature_detected!("avx");
    #[cfg(not(target_arch = "x86_64"))]
    let avx = false;
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        avx,
        "ArcanaGlyph запускается"
    );
    if let Some(dir) = CoreConfig::logs_dir() {
        tracing::info!("Логи пишутся в {}", dir.display());
    }
}

fn main() {
    // Раннее: --grant-portal subcommand. Должен сработать ДО Tauri-инициализации
    // (нам не нужны окна / трей / engine — только portal warmup).
    if std::env::args().any(|a| a == "--grant-portal") {
        setup::run_grant_portal_and_exit();
    }

    // Инициализируем логирование: stdout (для `make run`) + файл с ротацией по дням.
    // Файловый лог критичен для Windows-сборки (windows_subsystem = "windows" → нет
    // консоли, stdout теряется) — это единственный канал диагностики от пользователя
    // без dev-окружения. `_log_guard` держим живым до конца main(): при его drop'е
    // non-blocking appender дописывает буфер на диск.
    let _log_guard = init_logging();
    install_panic_hook();
    log_startup_diagnostics();

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

    // Парсим хоткеи в Shortcut для сравнения в handler. ВАЖНО: сравнивать строки
    // нельзя — Display плагина канонизирует регистр и имя клавиши ("Control+`" →
    // "control+Backquote"), поэтому raw-конфиг никогда не совпал бы с Display
    // пришедшего события. Сравнение распарсенных Shortcut'ов (mods+key) — это и
    // рекомендованный плагином паттерн, и ЕДИНСТВЕННЫЙ рабочий путь на Windows
    // (там нет GNOME-gsettings fallback'а, плагин — единственный источник событий).
    let trigger_shortcut: Arc<Option<tauri_plugin_global_shortcut::Shortcut>> = Arc::new(hotkey.parse().ok());
    let pause_shortcut: Arc<Option<tauri_plugin_global_shortcut::Shortcut>> = Arc::new(if hotkey_pause.is_empty() {
        None
    } else {
        hotkey_pause.parse().ok()
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Второй экземпляр — показываем окно первого
            tray::show_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler({
                    let trigger_shortcut = Arc::clone(&trigger_shortcut);
                    let pause_shortcut = Arc::clone(&pause_shortcut);
                    move |app, shortcut, event| {
                        if event.state() == ShortcutState::Pressed
                            && let Some(engine_state) = app.try_state::<EngineState>()
                            && let Some(engine) = engine_state.get()
                        {
                            if pause_shortcut.as_ref().as_ref() == Some(shortcut) {
                                tracing::info!(source = "tauri-shortcut", "Горячая клавиша паузы: {shortcut}");
                                engine.pause();
                            } else if trigger_shortcut.as_ref().as_ref() == Some(shortcut) {
                                tracing::info!(source = "tauri-shortcut", "Горячая клавиша триггера: {shortcut}");
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
            commands::update_install_ready,
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
