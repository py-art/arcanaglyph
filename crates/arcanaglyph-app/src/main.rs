// crates/arcanaglyph-app/src/main.rs

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod models;
mod net;
mod setup;
mod tray;
mod updater;

use arcanaglyph_core::CoreConfig;
use arcanaglyph_core::config::TranscriberType;
use commands::EngineState;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
use tauri_plugin_global_shortcut::ShortcutState;

/// Глобально удерживаемый `WorkerGuard` файлового лог-аппендера. Раньше держался
/// локальной переменной в `main()`, но при kill-по-сигналу `main()` не доходит до
/// конца — guard не дропается, и последние строки лога (в т.ч. сама причина выхода)
/// теряются в буфере non-blocking аппендера. Глобальное хранилище позволяет
/// флашить его из обработчика сигналов перед `process::exit`. См. [`flush_logs`].
static LOG_GUARD: Mutex<Option<tracing_appender::non_blocking::WorkerGuard>> = Mutex::new(None);

/// Принудительно дописывает буфер файлового лога на диск (через drop `WorkerGuard`).
/// Идемпотентна: повторный вызов — no-op (guard уже взят). Вызывается из обработчика
/// сигналов и в конце `main()` — кто первый, тот и флашит.
fn flush_logs() {
    if let Ok(mut g) = LOG_GUARD.lock() {
        // take() + неявный drop возвращённого значения = синхронный флаш буфера.
        let _ = g.take();
    }
}

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

/// Перехватывает терминирующие сигналы (SIGTERM/SIGINT/SIGHUP/SIGQUIT) в выделенном
/// потоке, логирует имя сигнала И ОТПРАВИТЕЛЯ (PID + `/proc/<pid>/comm`), флашит лог
/// и завершает процесс кодом `128 + N`. Зачем: приложение несколько раз «само»
/// закрывалось без следа в логах — apport ловит только фатальные SIGSEGV/SIGABRT,
/// а SIGTERM/SIGHUP уходят молча, и при kill-по-сигналу буфер non-blocking лога не
/// дописывается. Теперь в логе останется строка `target=signal` с именем убийцы —
/// это и есть инструмент для поиска причины спонтанных завершений.
///
/// SIGKILL/SIGSTOP перехватить невозможно (ядро): если виновник шлёт именно их,
/// строки в логе не будет — тогда отправителя ловим извне (`bpftrace`/`auditctl`).
#[cfg(unix)]
fn install_signal_logger() {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
    use signal_hook::iterator::SignalsInfo;
    use signal_hook::iterator::exfiltrator::origin::WithOrigin;

    // WithOrigin прокидывает siginfo: signal + отправитель (pid/uid), если ядро их дало.
    let mut signals = match SignalsInfo::<WithOrigin>::new([SIGTERM, SIGINT, SIGHUP, SIGQUIT]) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Не удалось установить диагностический обработчик сигналов: {e}");
            return;
        }
    };

    let spawned = std::thread::Builder::new().name("signal-diag".into()).spawn(move || {
        // Первого терминирующего сигнала достаточно — логируем его и выходим.
        if let Some(origin) = (&mut signals).into_iter().next() {
            let signum = origin.signal;
            let signame = signal_hook::low_level::signal_name(signum).unwrap_or("UNKNOWN");
            // Кто послал сигнал. `process` = None, когда сигнал от ядра/анонимный.
            let (sender_pid, sender_name) = match origin.process {
                Some(p) => {
                    let name = std::fs::read_to_string(format!("/proc/{}/comm", p.pid))
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|_| "<comm недоступен>".to_string());
                    (p.pid.to_string(), name)
                }
                None => ("<ядро/анонимно>".to_string(), "<нет данных>".to_string()),
            };
            tracing::error!(
                target: "signal",
                signal = signame,
                signum,
                sender_pid = %sender_pid,
                sender = %sender_name,
                "Получен терминирующий сигнал — приложение завершается ИЗВНЕ (не штатный выход)"
            );
            flush_logs();
            // 128 + N — конвенциональный код выхода shell для смерти по сигналу.
            std::process::exit(128 + signum);
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("Не удалось запустить поток диагностики сигналов: {e}");
    }
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

/// Определяет фактический STT-движок на старте с учётом двух runtime-ограничений
/// и (если был fallback) пару `(original, fallback)` для единого toast'а в UI.
///
/// Случай 1 — сохранённый движок не включён в текущую сборку (например, в БД лежит
/// Vosk, а собрано без feature `vosk`). Берём первый скомпилированный. В БД НЕ
/// сохраняем: пользовательский выбор остаётся первичным и снова станет активным
/// после пересборки с нужным feature-set'ом.
///
/// Случай 2 — ONNX-движок (GigaAM/Qwen3-ASR) на CPU без AVX. Эмпирически: Microsoft
/// pre-built ONNX Runtime (его тянет `ort` через `download-binaries`) на CPU без AVX
/// крашит SIGILL ещё до первого вывода. Мягко переключаемся на первый не-ONNX движок
/// (Whisper/Vosk). Если таких в сборке нет — оставляем как есть (engine упадёт SIGILL,
/// редкий кейс явной ошибки сборки) и логируем. `ort_needs_avx` передаётся параметром
/// (= `cfg!(feature="gigaam") && !cfg!(feature="gigaam-system-ort")`) ради тестируемости.
///
/// Чистая функция (детерминирована по входам, без I/O) — тестируема.
fn resolve_startup_engine(
    configured: TranscriberType,
    avx_ok: bool,
    ort_needs_avx: bool,
) -> (TranscriberType, Option<(String, String)>) {
    let mut transcriber = configured;
    let mut engine_fallback: Option<(String, String)> = None;

    // Случай 1: движок не включён в текущую сборку.
    if !transcriber.is_compiled_in() {
        let original = transcriber.as_str().to_string();
        transcriber = TranscriberType::compiled_engines()
            .into_iter()
            .next()
            .unwrap_or_default();
        let fallback = transcriber.as_str().to_string();
        tracing::warn!(
            "Движок '{}' не включён в эту сборку — используется '{}' (runtime-fallback, БД не меняется)",
            original,
            fallback
        );
        engine_fallback = Some((original, fallback));
    }

    // Случай 2: ONNX-based движок на CPU без AVX.
    let needs_avx = matches!(transcriber, TranscriberType::GigaAm | TranscriberType::Qwen3Asr) && ort_needs_avx;
    if !avx_ok && needs_avx {
        let original = transcriber.as_str().to_string();
        // Первый не-ONNX движок среди скомпилированных (Whisper, потом Vosk).
        let alt = TranscriberType::compiled_engines()
            .into_iter()
            .find(|t| !matches!(t, TranscriberType::GigaAm | TranscriberType::Qwen3Asr));
        if let Some(new_engine) = alt {
            transcriber = new_engine;
            let fallback = transcriber.as_str().to_string();
            tracing::warn!(
                "CPU без AVX: '{}' требует ONNX Runtime с AVX — runtime-переключение на '{}' (БД не меняется)",
                original,
                fallback
            );
            engine_fallback = Some((original, fallback));
        } else {
            tracing::error!("CPU без AVX и нет не-ONNX движков в сборке — engine может крашить");
        }
    }

    (transcriber, engine_fallback)
}

fn main() {
    // Раннее: --grant-portal subcommand. Должен сработать ДО Tauri-инициализации
    // (нам не нужны окна / трей / engine — только portal warmup).
    if std::env::args().any(|a| a == "--grant-portal") {
        setup::run_grant_portal_and_exit();
    }

    // Раннее: --trigger / --pause — короткоживущий клиент IPC-триггера, который
    // запускает нативный GNOME-хоткей вместо прежних bash-скриптов с `nc`. Шлём
    // одну датаграмму в Unix-сокет основного процесса и сразу выходим, НЕ поднимая
    // Tauri/трей/engine/ORT. Только Linux: на Win/macOS хоткей ловится in-process.
    #[cfg(target_os = "linux")]
    if let Some(cmd) = std::env::args().find_map(|a| match a.as_str() {
        "--trigger" => Some("trigger"),
        "--pause" => Some("pause"),
        _ => None,
    }) {
        setup::bootstrap::send_trigger_and_exit(cmd);
    }

    // Инициализируем логирование: stdout (для `make run`) + файл с ротацией по дням.
    // Файловый лог критичен для Windows-сборки (windows_subsystem = "windows" → нет
    // консоли, stdout теряется) — это единственный канал диагностики от пользователя
    // без dev-окружения. `_log_guard` держим живым до конца main(): при его drop'е
    // non-blocking appender дописывает буфер на диск.
    // Guard храним в глобале (а не в локальной переменной): обработчик сигналов
    // флашит его перед exit, иначе причина kill-по-сигналу теряется в буфере.
    if let Ok(mut g) = LOG_GUARD.lock() {
        *g = init_logging();
    }
    install_panic_hook();
    // Диагностика спонтанных завершений (Unix). Ставим сразу после логирования,
    // чтобы поймать сигнал на любом этапе старта.
    #[cfg(unix)]
    install_signal_logger();
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

    // Авто-fallback движка на старте (единый toast в UI): движок не включён в сборку,
    // либо ONNX-движок на CPU без AVX. Полная логика и обоснования — в doc
    // `resolve_startup_engine`. На не-x86_64 AVX-проблем нет (другие SIMD-наборы).
    #[cfg(target_arch = "x86_64")]
    let avx_ok = std::is_x86_feature_detected!("avx");
    #[cfg(not(target_arch = "x86_64"))]
    let avx_ok = true;
    let ort_needs_avx = cfg!(feature = "gigaam") && !cfg!(feature = "gigaam-system-ort");
    let (transcriber, engine_fallback) = resolve_startup_engine(config.transcriber, avx_ok, ort_needs_avx);
    config.transcriber = transcriber;

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

    let run_result = tauri::Builder::default()
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
        .run(tauri::generate_context!());
    // Штатный выход из event-loop (трей «Выход», апдейтер): флашим лог перед концом
    // процесса. При ошибке запуска — логируем её (panic-hook бы тоже поймал, но так
    // причина попадёт в файл до флаша).
    if let Err(e) = run_result {
        tracing::error!("Ошибка запуска/работы Tauri: {e}");
    }
    flush_logs();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_startup_engine_keeps_compiled_engine_when_avx_ok() {
        // GigaAM включён по умолчанию во всех наших сборках; AVX есть → ни случай 1,
        // ни случай 2 не срабатывают: движок не меняется, fallback отсутствует.
        let (engine, fallback) = resolve_startup_engine(TranscriberType::GigaAm, true, true);
        assert_eq!(engine, TranscriberType::GigaAm);
        assert!(fallback.is_none());
    }

    #[test]
    fn test_resolve_startup_engine_skips_avx_switch_when_ort_not_avx_bound() {
        // system-ort сборка (ort_needs_avx=false): даже на CPU без AVX случай 2 не
        // применяется — ONNX-движок работает через локальную libonnxruntime.so.
        let (engine, fallback) = resolve_startup_engine(TranscriberType::GigaAm, false, false);
        assert_eq!(engine, TranscriberType::GigaAm);
        assert!(fallback.is_none());
    }
}
