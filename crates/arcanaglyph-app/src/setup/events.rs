// crates/arcanaglyph-app/src/setup/events.rs
//
// Три фоновых spawn'а, которые крутятся всю жизнь приложения:
//   1. `run_engine_event_loop` — broadcast Receiver<EngineEvent> → tray-state +
//      виджет + Tauri events для frontend.
//   2. `spawn_update_checker` — фоновый раз-в-сутки чек GitHub releases с
//      exponential backoff на сетевых ошибках.
//   3. `spawn_trigger_listener` (только Linux) — Unix-сокет trigger.sock, в
//      который пишет короткоживущий `arcanaglyph --trigger`, запускаемый
//      нативным GNOME custom-keybinding'ом.

use crate::commands::EngineState;
use crate::tray;
use crate::updater;
#[cfg(target_os = "linux")]
use arcanaglyph_core::CoreConfig;
use arcanaglyph_core::EngineEvent;
use arcanaglyph_core::error::ApiError;
use arcanaglyph_core::history::HistoryDB;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::broadcast;

/// Маппит событие движка в `(имя Tauri-события, JSON-payload)` для emit во frontend.
/// `None` — событие не транслируется напрямую (`RequestFocus` обрабатывается в loop'е
/// через `show_window`). `Error` отдаёт типизированный `ApiError` (`{ kind, message,
/// hint }`) вместо плоского `{ message }`: UI по `kind` выбирает иконку, `hint`
/// показывает в toast'е, для `kind=cancelled` toast не показывается вовсе. Чистая
/// функция — тестируема без живого Tauri.
fn engine_event_to_emit(event: &EngineEvent) -> Option<(&'static str, serde_json::Value)> {
    Some(match event {
        EngineEvent::RecordingStarted => ("engine://recording-started", serde_json::json!({})),
        EngineEvent::RecordingPaused => ("engine://recording-paused", serde_json::json!({})),
        EngineEvent::RecordingResumed => ("engine://recording-resumed", serde_json::json!({})),
        EngineEvent::TranscriptionResult(text) => {
            ("engine://transcription-result", serde_json::json!({ "text": text }))
        }
        EngineEvent::Transcribing => ("engine://transcribing", serde_json::json!({})),
        EngineEvent::FinishedProcessing => ("engine://finished-processing", serde_json::json!({})),
        EngineEvent::ModelLoading(name) => ("engine://model-loading", serde_json::json!({ "model": name })),
        EngineEvent::ModelLoaded => ("engine://model-loaded", serde_json::json!({})),
        EngineEvent::Error(msg) => {
            let api_err = ApiError::from_message(msg);
            ("engine://error", serde_json::to_value(&api_err).unwrap_or_default())
        }
        EngineEvent::RequestFocus => return None,
    })
}

/// Event loop: пробрасывает события `ArcanaEngine` во фронтенд + обновляет tray
/// и видимость виджета записи. Завершается когда broadcast-channel закрыт.
pub async fn run_engine_event_loop(
    app_handle: AppHandle,
    engine_state: EngineState,
    mut rx: broadcast::Receiver<EngineEvent>,
) {
    loop {
        match rx.recv().await {
            Ok(event) => {
                update_tray_and_widget(&app_handle, &engine_state, &event);
                // RequestFocus не транслируется во frontend — это сигнал поднять окно.
                if matches!(event, EngineEvent::RequestFocus) {
                    tray::show_window(&app_handle);
                    continue;
                }
                if let Some((event_name, payload)) = engine_event_to_emit(&event) {
                    let _ = app_handle.emit(event_name, payload);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("Пропущено {} событий", n);
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}

/// Обновляет tray (текст/иконку записи) и видимость виджета в ответ на событие
/// движка. Side-effect поверх живого `AppHandle` (set_tray_*/get_webview_window) —
/// проверяется live-verify, не юнитом. Вынесено из `run_engine_event_loop` ради
/// снижения вложенности loop'а.
fn update_tray_and_widget(app_handle: &AppHandle, engine_state: &EngineState, event: &EngineEvent) {
    match event {
        EngineEvent::RecordingStarted | EngineEvent::RecordingResumed => {
            tray::set_tray_text(app_handle, "Остановить запись");
            tray::set_tray_recording(app_handle, true);
            // Показываем виджет записи (если включён в настройках)
            if engine_state.get().is_some_and(|e| e.show_widget())
                && let Some(w) = app_handle.get_webview_window("widget")
            {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        EngineEvent::RecordingPaused => {
            tray::set_tray_text(app_handle, "Продолжить запись");
            tray::set_tray_state(app_handle, tray::TrayState::Paused);
            // Виджет остаётся видимым при паузе
        }
        EngineEvent::Transcribing => {
            tray::set_tray_text(app_handle, "Транскрибация...");
            tray::set_tray_recording(app_handle, false);
            // Скрываем виджет — запись окончена
            if let Some(w) = app_handle.get_webview_window("widget") {
                let _ = w.hide();
            }
        }
        EngineEvent::FinishedProcessing => {
            tray::set_tray_text(app_handle, "Начать запись");
            tray::set_tray_recording(app_handle, false);
            // Скрываем виджет (страховка)
            if let Some(w) = app_handle.get_webview_window("widget") {
                let _ = w.hide();
            }
        }
        _ => {}
    }
}

/// Решает, пора ли делать сетевую проверку обновлений.
/// `force` (первый прогон после старта приложения) обходит 23-часовой throttle:
/// иначе «закрыл-открыл» вскоре после релиза не покажет баннер до истечения
/// суток с прошлой проверки. Периодические прогоны в loop'е остаются под гейтом
/// (раз в ~24ч), чтобы не жечь GitHub rate-limit (ETag → 304 на неизменном релизе).
fn update_check_due(last_check_at: Option<i64>, now: i64, force: bool) -> bool {
    force || last_check_at.map(|t| now - t >= 23 * 3600).unwrap_or(true)
}

/// Запускает фоновый чекер обновлений: первый запрос через 60с (даём engine
/// догрузиться, не конкурируем за сеть с download_model'ями) и делается ВСЕГДА —
/// в обход throttle, чтобы свежий релиз был виден сразу после перезапуска. Далее
/// раз в 24ч в loop'е под гейтом. На сетевых ошибках — exponential backoff до 7 дней.
pub fn spawn_update_checker(app_handle: AppHandle, history_db: Arc<HistoryDB>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let mut backoff = std::time::Duration::from_secs(86400);
        // Первый прогон после старта обходит throttle (см. update_check_due).
        let mut force_check = true;
        loop {
            let state = updater::read_state(&history_db);
            let now = updater::unix_now();
            if update_check_due(state.last_check_at, now, force_check) {
                // INFO, а не DEBUG: апдейтер раньше «молчал» в файловом логе
                // (нормальный путь был на DEBUG, лог пишется на INFO) — нельзя
                // было понять, ходил ли чекер вообще. Раз в сутки — не спам.
                tracing::info!("Проверка обновлений: запрос к GitHub Releases");
                match updater::check_for_update(&history_db).await {
                    Ok(Some(info)) => {
                        tracing::info!("Update available: {}", info.latest_version);
                        let _ = app_handle.emit("update://available", info);
                        backoff = std::time::Duration::from_secs(86400);
                    }
                    Ok(None) => {
                        tracing::info!("Проверка обновлений: новых релизов нет");
                        backoff = std::time::Duration::from_secs(86400);
                    }
                    Err(e) => {
                        tracing::warn!("Update check failed: {}", e);
                        backoff = (backoff * 2).min(std::time::Duration::from_secs(7 * 86400));
                    }
                }
            } else {
                tracing::debug!("Update check skipped (< 23h since last)");
            }
            force_check = false;
            tokio::time::sleep(backoff).await;
        }
    });
}

/// IPC-триггер для Linux (Wayland/GNOME): короткоживущий `arcanaglyph --trigger`,
/// запущенный нативным GNOME custom-keybinding'ом, шлёт датаграмму в Unix-сокет
/// `~/.config/arcanaglyph/trigger.sock`. Передаём команду в engine. Слушает до
/// завершения процесса. Заменяет прежний UDP :9002 + bash-скрипты с `nc` —
/// больше нет ни жёстко зашитого порта, ни внешних скриптов.
#[cfg(target_os = "linux")]
pub fn spawn_trigger_listener(engine_state: EngineState) {
    tauri::async_runtime::spawn(async move {
        let path = match CoreConfig::trigger_socket_path() {
            Some(p) => p,
            None => {
                tracing::error!("Не удалось определить путь IPC-сокета триггера");
                return;
            }
        };
        // Гарантируем директорию и снимаем устаревший сокет: bind упадёт, если
        // файл уже существует (например, после некорректного завершения прошлой
        // копии). single-instance гарантирует, что второго слушателя нет.
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::remove_file(&path).await;
        let socket = match tokio::net::UnixDatagram::bind(&path) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Не удалось привязать IPC-сокет {}: {e}", path.display());
                return;
            }
        };
        let mut buf = [0u8; 1024];
        tracing::info!("Слушаю IPC-триггеры на {}", path.display());
        loop {
            if let Ok(n) = socket.recv(&mut buf).await
                && let Some(engine) = engine_state.get()
            {
                let msg = String::from_utf8_lossy(&buf[0..n]);
                // Диагностика double-trigger: лог каждой датаграммы с содержимым.
                // Позволяет соотнести с tauri-shortcut логами и call_id из
                // engine.rs trigger().
                tracing::info!(source = "uds", payload = %msg.trim(), "IPC packet received");
                if msg.contains("pause") {
                    engine.pause();
                } else if msg.contains("trigger") {
                    engine.trigger();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{EngineEvent, engine_event_to_emit, update_check_due};

    const DAY: i64 = 24 * 3600;

    #[test]
    fn test_force_bypasses_throttle() {
        // Первый прогон после старта: проверяем даже если только что проверяли.
        let now = 100 * DAY;
        assert!(update_check_due(Some(now - 60), now, true));
    }

    #[test]
    fn test_no_state_checks_immediately() {
        // Нет записи о прошлой проверке — проверяем (даже без force).
        assert!(update_check_due(None, 100 * DAY, false));
    }

    #[test]
    fn test_throttled_within_23h() {
        // Прошло меньше 23ч и не force — пропускаем (это и есть тот самый гейт,
        // из-за которого свежий релиз не виден до суток без этого фикса).
        let now = 100 * DAY;
        assert!(!update_check_due(Some(now - 22 * 3600), now, false));
    }

    #[test]
    fn test_due_after_23h() {
        // Прошло ≥23ч — периодическая проверка срабатывает без force.
        let now = 100 * DAY;
        assert!(update_check_due(Some(now - 23 * 3600), now, false));
    }

    #[test]
    fn test_engine_event_to_emit_names_and_payloads() {
        // Простые события: имя задано, payload пустой объект.
        assert_eq!(
            engine_event_to_emit(&EngineEvent::RecordingStarted).unwrap().0,
            "engine://recording-started"
        );
        assert_eq!(
            engine_event_to_emit(&EngineEvent::ModelLoaded).unwrap().0,
            "engine://model-loaded"
        );

        // TranscriptionResult кладёт распознанный текст в payload.
        let (name, payload) = engine_event_to_emit(&EngineEvent::TranscriptionResult("привет".into())).unwrap();
        assert_eq!(name, "engine://transcription-result");
        assert_eq!(payload["text"], "привет");

        // ModelLoading кладёт имя модели.
        let (_, payload) = engine_event_to_emit(&EngineEvent::ModelLoading("gigaam".into())).unwrap();
        assert_eq!(payload["model"], "gigaam");
    }

    #[test]
    fn test_engine_event_to_emit_error_is_typed_and_focus_is_none() {
        // Error → типизированный ApiError payload (kind распознаётся из текста).
        let (name, payload) = engine_event_to_emit(&EngineEvent::Error("Ошибка сети: timeout".into())).unwrap();
        assert_eq!(name, "engine://error");
        assert_eq!(payload["kind"], "network");
        assert_eq!(payload["message"], "timeout");

        // RequestFocus не транслируется напрямую (обрабатывается в loop'е).
        assert!(engine_event_to_emit(&EngineEvent::RequestFocus).is_none());
    }
}
