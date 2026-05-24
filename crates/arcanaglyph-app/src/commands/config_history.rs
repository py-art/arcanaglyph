// crates/arcanaglyph-app/src/commands/config_history.rs
//
// Команды для работы с конфигом (load/save с применением hot-reload эффектов:
// autostart / tray visibility / widget position), фильтр истории, выбор языка,
// CRUD по истории транскрипций, экспорт, retranscribe, get_audio_data.

use super::{EngineState, get_engine};
use crate::setup::bootstrap::set_autostart;
use crate::tray;
use arcanaglyph_core::CoreConfig;
use arcanaglyph_core::config::TranscriberType;
use arcanaglyph_core::history::HistoryDB;
use std::sync::Arc;
use tauri::Manager;

/// Tauri-команда: загрузить текущую конфигурацию
#[tauri::command]
pub fn load_config() -> Result<serde_json::Value, String> {
    let config = CoreConfig::load().map_err(|e| e.to_string())?;
    serde_json::to_value(&config).map_err(|e| e.to_string())
}

/// Tauri-команда: сохранить конфигурацию и применить к движку
#[tauri::command]
pub fn save_config(
    config: serde_json::Value,
    app: tauri::AppHandle,
    engine: tauri::State<'_, EngineState>,
) -> Result<(), String> {
    let _ = get_engine; // silence unused warning when engine не используется в feature-set
    let config: CoreConfig = serde_json::from_value(config).map_err(|e| format!("Ошибка парсинга конфига: {}", e))?;
    config.save().map_err(|e| e.to_string())?;

    // Управляем автозапуском
    set_autostart(config.autostart);

    // Управляем видимостью трея
    tray::set_tray_visible(&app, config.show_tray);

    // Репозиционируем виджет если он создан — на лету, без рестарта приложения.
    // На Wayland set_position может быть проигнорирован mutter'ом — это ожидаемо.
    if let Some(w) = app.get_webview_window("widget")
        && let Ok(Some(monitor)) = w.primary_monitor()
    {
        let screen = monitor.size();
        let scale = monitor.scale_factor();
        let (x, y) = arcanaglyph_core::config::widget_position_xy(
            &config.widget_position,
            screen.width as f64 / scale,
            screen.height as f64 / scale,
            220.0,
            40.0,
        );
        let _ = w.set_position(tauri::LogicalPosition::new(x, y));
    }

    // Wayland-путь: пишем позицию в gsettings нашего GNOME-расширения.
    // Если расширение установлено и включено — оно сразу подхватит и переместит
    // окно. Если нет — gsettings вернёт ошибку, игнорируем (для X11/non-GNOME
    // эта запись просто бессмысленна, но безвредна).
    let _ = std::process::Command::new("gsettings")
        .args([
            "set",
            "org.gnome.shell.extensions.arcanaglyph-widget",
            "position",
            &config.widget_position,
        ])
        .output();

    if let Some(e) = engine.get() {
        let prev_transcriber = e.active_transcriber_type();
        let new_transcriber = config.transcriber.clone();
        e.update_config(config);

        // Eager-preload: если активный движок изменился — грузим новую модель в фоне
        // СРАЗУ (не дожидаясь первого Ctrl+Ё). preload_model сам эмитит ModelLoading и
        // ModelLoaded → frontend обновит top-status и блокирует mic-btn на время загрузки.
        // Это убирает тот баг, когда top-status горел «Готов» пока на самом деле модель
        // ещё не была в памяти и trigger() лениво её догружал на 10-20с.
        if prev_transcriber != new_transcriber {
            let engine_state: EngineState = engine.inner().clone();
            tauri::async_runtime::spawn(async move {
                let _ = tokio::task::spawn_blocking(move || {
                    if let Some(e) = engine_state.get()
                        && let Err(err) = e.preload_model(&new_transcriber)
                    {
                        tracing::warn!("Eager preload '{:?}' не удалась: {}", new_transcriber, err);
                    }
                })
                .await;
            });
        }
    }
    Ok(())
}

/// Tauri-команда: сохранить выбранный период фильтра истории (без применения к движку)
#[tauri::command]
pub fn set_history_filter(secs: u64) -> Result<(), String> {
    let mut cfg = CoreConfig::load().map_err(|e| e.to_string())?;
    cfg.history_filter_secs = secs;
    cfg.save().map_err(|e| e.to_string())
}

/// Tauri-команда: сохранить выбранный язык интерфейса (без применения к движку)
#[tauri::command]
pub fn set_language(lang: String) -> Result<(), String> {
    let mut cfg = CoreConfig::load().map_err(|e| e.to_string())?;
    cfg.language = lang;
    cfg.save().map_err(|e| e.to_string())
}

/// Tauri-команда: получить историю транскрибаций
#[tauri::command]
pub fn get_history(
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
pub fn delete_history_entry(recording_id: i64, db: tauri::State<'_, Arc<HistoryDB>>) -> Result<(), String> {
    db.delete_recording(recording_id).map_err(|e| e.to_string())
}

/// Tauri-команда: очистить всю историю
#[tauri::command]
pub fn clear_history(db: tauri::State<'_, Arc<HistoryDB>>) -> Result<(), String> {
    db.clear().map_err(|e| e.to_string())
}

/// Tauri-команда: экспорт истории в файл (txt или csv)
#[tauri::command]
pub fn export_history(format: String, db: tauri::State<'_, Arc<HistoryDB>>) -> Result<String, String> {
    let content = db.export(&format).map_err(|e| e.to_string())?;
    let ext = if format == "csv" { "csv" } else { "txt" };
    let date = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    let filename = format!("arcanaglyph-history-{}.{}", date, ext);

    // Сохраняем в ~/Downloads/ или ~/
    let dir = dirs::download_dir()
        .or_else(dirs::home_dir)
        .ok_or("Не удалось определить директорию для сохранения")?;
    let path = dir.join(&filename);
    std::fs::write(&path, &content).map_err(|e| format!("Ошибка записи файла: {}", e))?;

    Ok(path.to_string_lossy().to_string())
}

/// Tauri-команда: повторно транскрибировать запись другой моделью.
///
/// `allow`'ы нужны для сборок с уменьшенным набором features (например `--no-default-features`),
/// где после раннего возврата ошибки оставшийся код становится статически unreachable.
#[tauri::command]
#[allow(unreachable_code, unused_variables)]
pub async fn retranscribe(
    recording_id: i64,
    transcriber_type: String,
    db: tauri::State<'_, Arc<HistoryDB>>,
) -> Result<serde_json::Value, String> {
    #[cfg(any(feature = "gigaam", feature = "gigaam-system-ort"))]
    use arcanaglyph_core::gigaam::transcriber::GigaAmTranscriber;
    use arcanaglyph_core::transcriber::Transcriber;
    #[cfg(feature = "vosk")]
    use arcanaglyph_core::transcriber::VoskTranscriber;
    #[cfg(feature = "whisper")]
    use arcanaglyph_core::transcriber::WhisperTranscriber;

    // Ранний выход, если запрошенный движок не включён в текущую сборку.
    // Это убирает unreachable-предупреждения при сборках с уменьшенным набором features
    // и даёт пользователю понятную ошибку до чтения аудиофайла.
    if !TranscriberType::compiled_engines()
        .iter()
        .any(|e| e.as_str() == transcriber_type)
    {
        return Err(format!(
            "Движок '{}' недоступен в этой сборке — пересоберите с соответствующей cargo feature",
            transcriber_type
        ));
    }

    // Получаем запись из БД
    let entries = db.query(0, 1000, 0).map_err(|e| e.to_string())?.0;
    let entry = entries
        .iter()
        .find(|e| e.recording.id == recording_id)
        .ok_or("Запись не найдена")?;

    if !entry.audio_exists {
        return Err("Аудиофайл удалён — повторная транскрибация невозможна".to_string());
    }

    let audio_path = &entry.recording.audio_path;
    let config = arcanaglyph_core::CoreConfig::load().map_err(|e| e.to_string())?;

    // Загружаем аудио
    let raw_bytes = std::fs::read(audio_path).map_err(|e| format!("Не удалось прочитать аудио: {}", e))?;
    let samples: Vec<i16> = raw_bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    // Определяем имя модели
    let (model_name, t_type) = match transcriber_type.as_str() {
        "vosk" => {
            let name = config
                .model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "vosk".to_string());
            (name, "vosk".to_string())
        }
        "whisper" => {
            let name = config
                .whisper_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "whisper".to_string());
            (name, "whisper".to_string())
        }
        "gigaam" => {
            let name = config
                .gigaam_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "gigaam".to_string());
            (name, "gigaam".to_string())
        }
        "qwen3asr" => {
            let name = config
                .qwen3asr_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "qwen3asr".to_string());
            (name, "qwen3asr".to_string())
        }
        _ => return Err("Неизвестный тип транскрайбера".to_string()),
    };

    // Проверяем, нет ли уже транскрибации этой моделью
    let existing = db.get_transcriptions(recording_id).map_err(|e| e.to_string())?;
    if existing.iter().any(|t| t.model_name == model_name) {
        return Err(format!("Запись уже распознана моделью {}", model_name));
    }

    // Создаём транскрайбер.
    // Каждое плечо собирается только при включённой соответствующей feature.
    // Любая строка, не подобранная активными плечами (включая корректные имена движков,
    // не включённых в сборку), попадает в дефолтное плечо с понятной ошибкой.
    let (transcriber, sr): (Box<dyn Transcriber>, u32) = match transcriber_type.as_str() {
        #[cfg(feature = "vosk")]
        "vosk" => {
            let t = VoskTranscriber::new(&config.model_path, config.sample_rate as f32).map_err(|e| e.to_string())?;
            (Box::new(t), config.sample_rate)
        }
        #[cfg(feature = "whisper")]
        "whisper" => {
            let t = WhisperTranscriber::new(&config.whisper_model_path).map_err(|e| e.to_string())?;
            (Box::new(t), config.sample_rate)
        }
        #[cfg(any(feature = "gigaam", feature = "gigaam-system-ort"))]
        "gigaam" => {
            let t = GigaAmTranscriber::new(&config.gigaam_model_path).map_err(|e| e.to_string())?;
            (Box::new(t), config.sample_rate)
        }
        #[cfg(feature = "qwen3asr")]
        "qwen3asr" => {
            let t = arcanaglyph_core::qwen3asr::transcriber::Qwen3AsrTranscriber::new(&config.qwen3asr_model_path)
                .map_err(|e| e.to_string())?;
            (Box::new(t), config.sample_rate)
        }
        other => {
            return Err(format!(
                "Движок '{}' недоступен в этой сборке — пересоберите с соответствующей cargo feature",
                other
            ));
        }
    };

    // Транскрибируем
    let text = tokio::task::spawn_blocking(move || transcriber.transcribe(&samples, sr))
        .await
        .map_err(|e| format!("{:?}", e))?
        .map_err(|e| e.to_string())?;

    if text.is_empty() {
        return Err("Распознавание вернуло пустой результат".to_string());
    }

    // Сохраняем в БД
    db.add_transcription(recording_id, &text, &model_name, &t_type)
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({ "text": text, "model_name": model_name }))
}

/// Tauri-команда: получить аудиоданные записи для воспроизведения (base64)
#[tauri::command]
pub fn get_audio_data(recording_id: i64, db: tauri::State<'_, Arc<HistoryDB>>) -> Result<serde_json::Value, String> {
    use base64::Engine;

    let entries = db.query(0, 100000, 0).map_err(|e| e.to_string())?.0;
    let entry = entries
        .iter()
        .find(|e| e.recording.id == recording_id)
        .ok_or("Запись не найдена")?;

    if !entry.audio_exists {
        return Err("Аудиофайл удалён".to_string());
    }

    let raw_bytes =
        std::fs::read(&entry.recording.audio_path).map_err(|e| format!("Не удалось прочитать аудио: {}", e))?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);

    let config = CoreConfig::load().map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "data": b64,
        "sample_rate": config.sample_rate,
    }))
}
