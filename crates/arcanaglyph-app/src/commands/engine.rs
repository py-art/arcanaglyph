// crates/arcanaglyph-app/src/commands/engine.rs
//
// Тонкие Tauri-команды над `ArcanaEngine`: запись (trigger/pause), отмена,
// статусы, уровень громкости. Все обращения идут через `get_engine` —
// до загрузки модели команды возвращают ошибку «Модель загружается...».

use super::{EngineState, get_engine};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Tauri-команда: переключатель записи (старт/стоп)
#[tauri::command]
pub async fn trigger(engine: tauri::State<'_, EngineState>) -> Result<(), String> {
    get_engine(&engine)?.trigger();
    Ok(())
}

/// Tauri-команда: переключатель паузы
#[tauri::command]
pub async fn pause(engine: tauri::State<'_, EngineState>) -> Result<(), String> {
    get_engine(&engine)?.pause();
    Ok(())
}

/// Tauri-команда: отменить текущую транскрибацию (только Whisper). Возвращает
/// `true` если активный движок поддерживает cancel и сигнал отправлен; `false`
/// если нет (Vosk / GigaAM / Qwen3-ASR — там нет API для прерывания инференса).
#[tauri::command]
pub async fn cancel_transcription(engine: tauri::State<'_, EngineState>) -> Result<bool, String> {
    Ok(get_engine(&engine)?.cancel_transcription())
}

/// Tauri-команда: поддерживает ли активный движок отмену. UI использует это,
/// чтобы показывать / скрывать кнопку «Стоп» в transcribing-состоянии.
#[tauri::command]
pub async fn active_supports_cancel(engine: tauri::State<'_, EngineState>) -> Result<bool, String> {
    Ok(get_engine(&engine)?.active_supports_cancel())
}

/// Tauri-команда: получить уровень громкости (0-100)
#[tauri::command]
pub fn get_audio_level(engine: tauri::State<'_, EngineState>) -> u32 {
    engine.get().map_or(0, |e| e.get_audio_level())
}

/// Tauri-команда: проверить, идёт ли запись
#[tauri::command]
pub async fn is_recording(engine: tauri::State<'_, EngineState>) -> Result<bool, String> {
    Ok(get_engine(&engine)?.is_recording().await)
}

/// Tauri-команда: проверить, на паузе ли запись
#[tauri::command]
pub async fn is_paused(engine: tauri::State<'_, EngineState>) -> Result<bool, String> {
    Ok(get_engine(&engine)?.is_paused().await)
}

/// Tauri-команда: проверить, загружена ли модель
#[tauri::command]
pub fn is_model_loaded(engine: tauri::State<'_, EngineState>) -> bool {
    engine.get().is_some()
}

/// Tauri-команда: получить список загруженных моделей + idle-секунды каждой.
/// `idle_seconds` нужно для отображения в Settings → «Выгружать неактивные модели…»:
/// UI показывает текущий простой каждой модели в пуле, чтобы пользователь понимал,
/// сколько ему осталось до выгрузки.
#[tauri::command]
pub fn get_loaded_models(engine: tauri::State<'_, EngineState>) -> Result<serde_json::Value, String> {
    let e = get_engine(&engine)?;
    Ok(serde_json::json!({
        "loaded": e.loaded_models(),
        "active": e.active_model_name(),
        "idle_seconds": e.loaded_models_idle_seconds(),
    }))
}

/// Tauri-команда: скрыть окно в трей и обновить флаг видимости
#[tauri::command]
pub async fn hide_window(window: tauri::Window, visible: tauri::State<'_, Arc<AtomicBool>>) -> Result<(), String> {
    let _ = window.hide();
    visible.store(false, Ordering::Relaxed);
    Ok(())
}
