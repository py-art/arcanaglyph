// crates/arcanaglyph-app/src/main.rs

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tray;

use arcanaglyph_core::{ArcanaEngine, CoreConfig, EngineEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

/// Tauri-команда: переключатель записи (старт/стоп)
#[tauri::command]
async fn trigger(engine: tauri::State<'_, Arc<ArcanaEngine>>) -> Result<(), String> {
    engine.trigger();
    Ok(())
}

/// Tauri-команда: переключатель паузы
#[tauri::command]
async fn pause(engine: tauri::State<'_, Arc<ArcanaEngine>>) -> Result<(), String> {
    engine.pause();
    Ok(())
}

/// Tauri-команда: получить уровень громкости (0-100)
#[tauri::command]
fn get_audio_level(engine: tauri::State<'_, Arc<ArcanaEngine>>) -> u32 {
    engine.get_audio_level()
}

/// Tauri-команда: проверить, идёт ли запись
#[tauri::command]
async fn is_recording(engine: tauri::State<'_, Arc<ArcanaEngine>>) -> Result<bool, String> {
    Ok(engine.is_recording().await)
}

/// Tauri-команда: загрузить текущую конфигурацию
#[tauri::command]
fn load_config() -> Result<serde_json::Value, String> {
    let config = CoreConfig::load().map_err(|e| e.to_string())?;
    serde_json::to_value(&config).map_err(|e| e.to_string())
}

/// Tauri-команда: сохранить конфигурацию и применить к движку
#[tauri::command]
fn save_config(config: serde_json::Value, engine: tauri::State<'_, Arc<ArcanaEngine>>) -> Result<(), String> {
    let config: CoreConfig = serde_json::from_value(config).map_err(|e| format!("Ошибка парсинга конфига: {}", e))?;
    config.save().map_err(|e| e.to_string())?;
    engine.update_config(config);
    Ok(())
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
                        if let Some(engine) = app.try_state::<Arc<ArcanaEngine>>() {
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

            // Создаём engine внутри async runtime, чтобы Handle::try_current() сработал
            let engine = tauri::async_runtime::block_on(async { ArcanaEngine::new(config, window_visible) })
                .map_err(|e| e.to_string())?;
            let engine = Arc::new(engine);
            app.manage(engine.clone());

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
            let engine_udp = engine.clone();
            tauri::async_runtime::spawn(async move {
                let udp_socket = tokio::net::UdpSocket::bind("127.0.0.1:9002")
                    .await
                    .expect("Не удалось привязать UDP :9002");
                let mut buf = [0u8; 1024];
                tracing::info!("Слушаю UDP-триггеры на порту 9002");
                loop {
                    if let Ok((n, _)) = udp_socket.recv_from(&mut buf).await {
                        let msg = String::from_utf8_lossy(&buf[0..n]);
                        if msg.contains("pause") {
                            engine_udp.pause();
                        } else if msg.contains("trigger") {
                            engine_udp.trigger();
                        }
                    }
                }
            });

            // Пробрасываем события engine → фронтенд через Tauri events
            let app_handle = app.handle().clone();
            let mut rx = engine.subscribe();
            tauri::async_runtime::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            // Обновляем текст и иконку в трее
                            match &event {
                                EngineEvent::RecordingStarted | EngineEvent::RecordingResumed => {
                                    tray::set_tray_text(&app_handle, "Остановить запись");
                                    tray::set_tray_recording(&app_handle, true);
                                }
                                EngineEvent::RecordingPaused => {
                                    tray::set_tray_text(&app_handle, "Продолжить запись");
                                }
                                EngineEvent::Transcribing => {
                                    tray::set_tray_text(&app_handle, "Транскрибация...");
                                    tray::set_tray_recording(&app_handle, false);
                                }
                                EngineEvent::FinishedProcessing => {
                                    tray::set_tray_text(&app_handle, "Начать запись");
                                    tray::set_tray_recording(&app_handle, false);
                                }
                                _ => {}
                            }

                            let (event_name, payload) = match &event {
                                EngineEvent::RecordingStarted => ("engine://recording-started", serde_json::json!({})),
                                EngineEvent::RecordingPaused => ("engine://recording-paused", serde_json::json!({})),
                                EngineEvent::RecordingResumed => ("engine://recording-resumed", serde_json::json!({})),
                                EngineEvent::TranscriptionResult(text) => {
                                    ("engine://transcription-result", serde_json::json!({"text": text}))
                                }
                                EngineEvent::Transcribing => {
                                    ("engine://transcribing", serde_json::json!({}))
                                }
                                EngineEvent::FinishedProcessing => {
                                    ("engine://finished-processing", serde_json::json!({}))
                                }
                                EngineEvent::RequestFocus => {
                                    // Выводим окно на передний план
                                    tray::show_window(&app_handle);
                                    continue;
                                }
                                EngineEvent::Error(msg) => {
                                    ("engine://error", serde_json::json!({"message": msg}))
                                }
                            };
                            let _ = app_handle.emit(event_name, payload);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("Пропущено {} событий", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::info!("Канал событий закрыт, завершаю слушатель.");
                            break;
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![trigger, pause, get_audio_level, is_recording, hide_window, load_config, save_config])
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
