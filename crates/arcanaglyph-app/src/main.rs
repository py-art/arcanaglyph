// crates/arcanaglyph-app/src/main.rs

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tray;

use arcanaglyph_core::{ArcanaEngine, CoreConfig, EngineEvent};
use arcanaglyph_core::history::HistoryDB;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Тип state для engine — инициализируется в фоне после показа окна
type EngineState = Arc<OnceLock<ArcanaEngine>>;

/// Получить engine или вернуть ошибку "модель загружается"
fn get_engine(state: &EngineState) -> Result<&ArcanaEngine, String> {
    state.get().ok_or_else(|| "Модель загружается...".to_string())
}

/// Tauri-команда: переключатель записи (старт/стоп)
#[tauri::command]
async fn trigger(engine: tauri::State<'_, EngineState>) -> Result<(), String> {
    get_engine(&engine)?.trigger();
    Ok(())
}

/// Tauri-команда: переключатель паузы
#[tauri::command]
async fn pause(engine: tauri::State<'_, EngineState>) -> Result<(), String> {
    get_engine(&engine)?.pause();
    Ok(())
}

/// Tauri-команда: получить уровень громкости (0-100)
#[tauri::command]
fn get_audio_level(engine: tauri::State<'_, EngineState>) -> u32 {
    engine.get().map_or(0, |e| e.get_audio_level())
}

/// Tauri-команда: проверить, идёт ли запись
#[tauri::command]
async fn is_recording(engine: tauri::State<'_, EngineState>) -> Result<bool, String> {
    Ok(get_engine(&engine)?.is_recording().await)
}

/// Tauri-команда: проверить, загружена ли модель
#[tauri::command]
fn is_model_loaded(engine: tauri::State<'_, EngineState>) -> bool {
    engine.get().is_some()
}

/// Tauri-команда: загрузить текущую конфигурацию
#[tauri::command]
fn load_config() -> Result<serde_json::Value, String> {
    let config = CoreConfig::load().map_err(|e| e.to_string())?;
    serde_json::to_value(&config).map_err(|e| e.to_string())
}

/// Tauri-команда: сохранить конфигурацию и применить к движку
#[tauri::command]
fn save_config(config: serde_json::Value, engine: tauri::State<'_, EngineState>) -> Result<(), String> {
    let config: CoreConfig = serde_json::from_value(config).map_err(|e| format!("Ошибка парсинга конфига: {}", e))?;
    config.save().map_err(|e| e.to_string())?;
    if let Some(e) = engine.get() {
        e.update_config(config);
    }
    Ok(())
}

/// Tauri-команда: получить историю транскрибаций
#[tauri::command]
fn get_history(
    since_secs: u64,
    limit: u32,
    offset: u32,
    db: tauri::State<'_, Arc<HistoryDB>>,
) -> Result<serde_json::Value, String> {
    let since_timestamp = if since_secs == 0 {
        0 // Все записи
    } else {
        chrono::Utc::now().timestamp() - since_secs as i64
    };
    let (entries, total) = db.query(since_timestamp, limit, offset).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "entries": entries, "total": total }))
}

/// Tauri-команда: удалить запись из истории
#[tauri::command]
fn delete_history_entry(recording_id: i64, db: tauri::State<'_, Arc<HistoryDB>>) -> Result<(), String> {
    db.delete_recording(recording_id).map_err(|e| e.to_string())
}

/// Tauri-команда: очистить всю историю
#[tauri::command]
fn clear_history(db: tauri::State<'_, Arc<HistoryDB>>) -> Result<(), String> {
    db.clear().map_err(|e| e.to_string())
}

/// Tauri-команда: повторно транскрибировать запись другой моделью
#[tauri::command]
async fn retranscribe(
    recording_id: i64,
    transcriber_type: String,
    db: tauri::State<'_, Arc<HistoryDB>>,
) -> Result<serde_json::Value, String> {
    use arcanaglyph_core::transcriber::{VoskTranscriber, WhisperTranscriber, Transcriber};

    // Получаем запись из БД
    let entries = db.query(0, 1000, 0).map_err(|e| e.to_string())?.0;
    let entry = entries.iter().find(|e| e.recording.id == recording_id)
        .ok_or("Запись не найдена")?;

    if !entry.audio_exists {
        return Err("Аудиофайл удалён — повторная транскрибация невозможна".to_string());
    }

    let audio_path = &entry.recording.audio_path;
    let config = arcanaglyph_core::CoreConfig::load().map_err(|e| e.to_string())?;

    // Загружаем аудио
    let raw_bytes = std::fs::read(audio_path).map_err(|e| format!("Не удалось прочитать аудио: {}", e))?;
    let samples: Vec<i16> = raw_bytes.chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    // Определяем имя модели
    let (model_name, t_type) = match transcriber_type.as_str() {
        "vosk" => {
            let name = config.model_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "vosk".to_string());
            (name, "vosk".to_string())
        }
        "whisper" => {
            let name = config.whisper_model_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "whisper".to_string());
            (name, "whisper".to_string())
        }
        _ => return Err("Неизвестный тип транскрайбера".to_string()),
    };

    // Проверяем, нет ли уже транскрибации этой моделью
    let existing = db.get_transcriptions(recording_id).map_err(|e| e.to_string())?;
    if existing.iter().any(|t| t.model_name == model_name) {
        return Err(format!("Запись уже распознана моделью {}", model_name));
    }

    // Создаём транскрайбер
    let (transcriber, sr): (Box<dyn Transcriber>, u32) = match transcriber_type.as_str() {
        "vosk" => {
            let t = VoskTranscriber::new(&config.model_path, config.sample_rate as f32).map_err(|e| e.to_string())?;
            (Box::new(t), config.sample_rate)
        }
        "whisper" => {
            let t = WhisperTranscriber::new(&config.whisper_model_path).map_err(|e| e.to_string())?;
            (Box::new(t), config.sample_rate)
        }
        _ => unreachable!(),
    };

    // Транскрибируем
    let text = tokio::task::spawn_blocking(move || {
        transcriber.transcribe(&samples, sr)
    }).await.map_err(|e| format!("{:?}", e))?.map_err(|e| e.to_string())?;

    if text.is_empty() {
        return Err("Распознавание вернуло пустой результат".to_string());
    }

    // Сохраняем в БД
    db.add_transcription(recording_id, &text, &model_name, &t_type).map_err(|e| e.to_string())?;

    Ok(serde_json::json!({ "text": text, "model_name": model_name }))
}

/// Tauri-команда: получить аудиоданные записи для воспроизведения (base64)
#[tauri::command]
fn get_audio_data(recording_id: i64, db: tauri::State<'_, Arc<HistoryDB>>) -> Result<serde_json::Value, String> {
    use base64::Engine;

    let entries = db.query(0, 100000, 0).map_err(|e| e.to_string())?.0;
    let entry = entries.iter().find(|e| e.recording.id == recording_id)
        .ok_or("Запись не найдена")?;

    if !entry.audio_exists {
        return Err("Аудиофайл удалён".to_string());
    }

    let raw_bytes = std::fs::read(&entry.recording.audio_path)
        .map_err(|e| format!("Не удалось прочитать аудио: {}", e))?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);

    let config = CoreConfig::load().map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "data": b64,
        "sample_rate": config.sample_rate,
    }))
}

/// Tauri-команда: скрыть окно в трей и обновить флаг видимости
#[tauri::command]
async fn hide_window(
    window: tauri::Window,
    visible: tauri::State<'_, Arc<AtomicBool>>,
) -> Result<(), String> {
    let _ = window.hide();
    visible.store(false, Ordering::Relaxed);
    Ok(())
}

fn main() {
    // Инициализируем логирование
    // Подавляем логи whisper.cpp (whisper_rs::whisper_sys_log) — оставляем только наши
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::new("info,whisper_rs=warn"),
        )
        .init();

    let config = CoreConfig::load().unwrap_or_else(|e| {
        tracing::warn!("Не удалось загрузить конфиг: {}, используем дефолтные настройки", e);
        CoreConfig::default()
    });
    let hotkey = config.hotkey.clone();

    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        tracing::info!("Нажата горячая клавиша: {:?}", shortcut);
                        if let Some(engine_state) = app.try_state::<EngineState>()
                            && let Some(engine) = engine_state.get()
                        {
                            engine.trigger();
                        }
                    }
                })
                .build(),
        )
        .setup(move |app| {
            // Флаг видимости окна: true при старте (окно видимо)
            let window_visible = Arc::new(AtomicBool::new(true));
            app.manage(window_visible.clone());

            // Engine создаётся в фоне — окно показывается сразу
            let engine_state: EngineState = Arc::new(OnceLock::new());
            app.manage(engine_state.clone());

            // Временная HistoryDB до загрузки engine
            let db_path = CoreConfig::history_db_path()
                .ok_or_else(|| "Не удалось определить путь БД".to_string())?;
            let audio_cache = CoreConfig::audio_cache_dir()
                .ok_or_else(|| "Не удалось определить путь кэша".to_string())?;
            let history_db = Arc::new(HistoryDB::new(&db_path, audio_cache).map_err(|e| e.to_string())?);
            app.manage(history_db);

            // Загрузка модели в фоне
            let app_handle_load = app.handle().clone();
            let engine_state_load = engine_state.clone();
            tauri::async_runtime::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    ArcanaEngine::new(config, window_visible)
                }).await;

                match result {
                    Ok(Ok(engine)) => {
                        // Подписываемся на события ПЕРЕД set, пока есть ownership
                        let mut rx = engine.subscribe();
                        let _ = engine_state_load.set(engine);
                        tracing::info!("Engine готов к работе");
                        let _ = app_handle_load.emit("engine://model-loaded", serde_json::json!({}));

                        // Event loop: пробрасываем события engine → фронтенд
                        let app_handle_events = app_handle_load.clone();
                        tokio::spawn(async move {
                            loop {
                                match rx.recv().await {
                                    Ok(event) => {
                                        match &event {
                                            EngineEvent::RecordingStarted | EngineEvent::RecordingResumed => {
                                                tray::set_tray_text(&app_handle_events, "Остановить запись");
                                                tray::set_tray_recording(&app_handle_events, true);
                                            }
                                            EngineEvent::RecordingPaused => {
                                                tray::set_tray_text(&app_handle_events, "Продолжить запись");
                                            }
                                            EngineEvent::Transcribing => {
                                                tray::set_tray_text(&app_handle_events, "Транскрибация...");
                                                tray::set_tray_recording(&app_handle_events, false);
                                            }
                                            EngineEvent::FinishedProcessing => {
                                                tray::set_tray_text(&app_handle_events, "Начать запись");
                                                tray::set_tray_recording(&app_handle_events, false);
                                            }
                                            _ => {}
                                        }
                                        let (event_name, payload) = match &event {
                                            EngineEvent::RecordingStarted => ("engine://recording-started", serde_json::json!({})),
                                            EngineEvent::RecordingPaused => ("engine://recording-paused", serde_json::json!({})),
                                            EngineEvent::RecordingResumed => ("engine://recording-resumed", serde_json::json!({})),
                                            EngineEvent::TranscriptionResult(text) => ("engine://transcription-result", serde_json::json!({"text": text})),
                                            EngineEvent::Transcribing => ("engine://transcribing", serde_json::json!({})),
                                            EngineEvent::FinishedProcessing => ("engine://finished-processing", serde_json::json!({})),
                                            EngineEvent::ModelLoaded => ("engine://model-loaded", serde_json::json!({})),
                                            EngineEvent::RequestFocus => { tray::show_window(&app_handle_events); continue; }
                                            EngineEvent::Error(msg) => ("engine://error", serde_json::json!({"message": msg})),
                                        };
                                        let _ = app_handle_events.emit(event_name, payload);
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                        tracing::warn!("Пропущено {} событий", n);
                                    }
                                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                                }
                            }
                        });
                    }
                    Ok(Err(e)) => {
                        tracing::error!("Ошибка создания engine: {}", e);
                        let _ = app_handle_load.emit("engine://error", serde_json::json!({"message": format!("Ошибка загрузки модели: {}", e)}));
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

            // Регистрируем глобальную горячую клавишу
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

            // UDP-триггер для Wayland (внешний скрипт ag-trigger → UDP :9002)
            let engine_udp = engine_state.clone();
            tauri::async_runtime::spawn(async move {
                let udp_socket = tokio::net::UdpSocket::bind("127.0.0.1:9002")
                    .await
                    .expect("Не удалось привязать UDP :9002");
                let mut buf = [0u8; 1024];
                tracing::info!("Слушаю UDP-триггеры на порту 9002");
                loop {
                    if let Ok((n, _)) = udp_socket.recv_from(&mut buf).await
                        && let Some(engine) = engine_udp.get()
                    {
                        let msg = String::from_utf8_lossy(&buf[0..n]);
                        if msg.contains("pause") {
                            engine.pause();
                        } else if msg.contains("trigger") {
                            engine.trigger();
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![trigger, pause, get_audio_level, is_recording, is_model_loaded, hide_window, load_config, save_config, get_history, delete_history_entry, clear_history, retranscribe, get_audio_data])
        .on_window_event(|window, event| {
            // Перехватываем закрытие окна — скрываем в трей вместо закрытия
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                if let Some(vis) = window.app_handle().try_state::<Arc<AtomicBool>>() {
                    vis.store(false, Ordering::Relaxed);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("Ошибка запуска Tauri");
}
