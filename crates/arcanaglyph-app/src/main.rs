// crates/arcanaglyph-app/src/main.rs

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tray;

use arcanaglyph_core::{ArcanaEngine, CoreConfig, EngineEvent};
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

/// Tauri-команда: проверить, идёт ли запись
#[tauri::command]
async fn is_recording(engine: tauri::State<'_, Arc<ArcanaEngine>>) -> Result<bool, String> {
    Ok(engine.is_recording().await)
}

fn main() {
    // Инициализируем логирование
    tracing_subscriber::fmt::init();

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
            // Создаём engine внутри async runtime, чтобы Handle::try_current() сработал
            let engine =
                tauri::async_runtime::block_on(async { ArcanaEngine::new(config) }).map_err(|e| e.to_string())?;
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
                            // Обновляем текст пункта меню в трее
                            match &event {
                                EngineEvent::RecordingStarted | EngineEvent::RecordingResumed => {
                                    tray::set_tray_text(&app_handle, "Остановить запись");
                                }
                                EngineEvent::RecordingPaused => {
                                    tray::set_tray_text(&app_handle, "Продолжить запись");
                                }
                                EngineEvent::FinishedProcessing => {
                                    tray::set_tray_text(&app_handle, "Начать запись");
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
                                EngineEvent::FinishedProcessing => {
                                    ("engine://finished-processing", serde_json::json!({}))
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
        .invoke_handler(tauri::generate_handler![trigger, pause, is_recording])
        .on_window_event(|window, event| {
            // Перехватываем закрытие окна — скрываем в трей вместо закрытия
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("Ошибка запуска Tauri");
}
