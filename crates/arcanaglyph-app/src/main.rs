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

/// Tauri-команда: проверить, на паузе ли запись
#[tauri::command]
async fn is_paused(engine: tauri::State<'_, EngineState>) -> Result<bool, String> {
    Ok(get_engine(&engine)?.is_paused().await)
}

/// Tauri-команда: проверить, загружена ли модель
#[tauri::command]
fn is_model_loaded(engine: tauri::State<'_, EngineState>) -> bool {
    engine.get().is_some()
}

/// Tauri-команда: получить список загруженных моделей
#[tauri::command]
fn get_loaded_models(engine: tauri::State<'_, EngineState>) -> Result<serde_json::Value, String> {
    let e = get_engine(&engine)?;
    Ok(serde_json::json!({
        "loaded": e.loaded_models(),
        "active": e.active_model_name(),
    }))
}

/// Управляет автозапуском через .desktop файл в ~/.config/autostart/
fn set_autostart(enabled: bool) {
    let home = match std::env::var("HOME") {
        Ok(h) => std::path::PathBuf::from(h),
        Err(_) => return,
    };
    let autostart_dir = home.join(".config/autostart");
    let desktop_file = autostart_dir.join("arcanaglyph.desktop");

    if enabled {
        let _ = std::fs::create_dir_all(&autostart_dir);

        // Определяем путь к исполняемому файлу
        let exec_path = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "arcanaglyph-app".to_string());

        let content = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=ArcanaGlyph\n\
             Comment=Голосовой ввод текста\n\
             Exec={}\n\
             Icon=arcanaglyph\n\
             Terminal=false\n\
             Categories=Utility;Audio;\n\
             X-GNOME-Autostart-enabled=true\n",
            exec_path
        );

        if let Err(e) = std::fs::write(&desktop_file, content) {
            tracing::warn!("Не удалось создать autostart: {}", e);
        } else {
            tracing::info!("Автозапуск включён: {}", desktop_file.display());
        }
    } else if desktop_file.exists() {
        let _ = std::fs::remove_file(&desktop_file);
        tracing::info!("Автозапуск отключён");
    }
}

/// Устанавливает UDP-скрипты ag-trigger и ag-pause (для Wayland)
fn install_wayland_scripts() {
    let is_wayland = std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false);
    if !is_wayland {
        return;
    }

    let bin_dir = match CoreConfig::scripts_dir() {
        Some(d) => d,
        None => return,
    };
    let _ = std::fs::create_dir_all(&bin_dir);

    let scripts = [
        ("ag-trigger", "#!/bin/bash\n# ArcanaGlyph: UDP-триггер записи\necho \"trigger\" | /usr/bin/nc -u -w0 127.0.0.1 9002\n"),
        ("ag-pause", "#!/bin/bash\n# ArcanaGlyph: UDP-триггер паузы\necho \"pause\" | /usr/bin/nc -u -w0 127.0.0.1 9002\n"),
    ];

    for (name, content) in &scripts {
        let path = bin_dir.join(name);
        if !path.exists() {
            if let Err(e) = std::fs::write(&path, content) {
                tracing::warn!("Не удалось создать {}: {}", path.display(), e);
                continue;
            }
            // chmod +x
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
            }
            tracing::info!("Установлен скрипт: {}", path.display());
        }
    }
}

/// Скачивание одного файла с прогрессом
async fn download_file(
    url: &str,
    dest: &std::path::Path,
    model_id: &str,
    file_idx: usize,
    total_files: usize,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    use futures_util::StreamExt;
    use tauri::Emitter;

    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let filename = dest.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    tracing::info!("Скачивание [{}/{}] {} → {}", file_idx + 1, total_files, filename, dest.display());

    let response = reqwest::get(url).await.map_err(|e| format!("Ошибка запроса {}: {}", filename, e))?;
    let total_size = response.content_length().unwrap_or(0);

    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(dest).await
        .map_err(|e| format!("Не удалось создать {}: {}", filename, e))?;

    let mut downloaded: u64 = 0;
    let mut last_progress: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Ошибка скачивания {}: {}", filename, e))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await
            .map_err(|e| format!("Ошибка записи {}: {}", filename, e))?;

        downloaded += chunk.len() as u64;
        let progress_pct = if total_size > 0 { downloaded * 100 / total_size } else { 0 };
        if progress_pct != last_progress {
            last_progress = progress_pct;
            let _ = app.emit("download://progress", serde_json::json!({
                "model_id": model_id,
                "file": filename,
                "file_idx": file_idx,
                "total_files": total_files,
                "downloaded": downloaded,
                "total": total_size,
                "percent": progress_pct,
            }));
        }
    }
    Ok(())
}

/// Tauri-команда: скачать модель (один или несколько файлов) с прогрессом
#[tauri::command]
async fn download_model(
    model_id: String,
    url: String,
    dest_dir: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    use tauri::Emitter;

    let dest_path = std::path::PathBuf::from(&dest_dir);
    let _ = std::fs::create_dir_all(&dest_path);

    // Находим модель в реестре для extra_files
    let model_info = arcanaglyph_core::transcription_models::find(&model_id);
    let extra_files: Vec<(&str, &str)> = model_info
        .and_then(|m| m.extra_files)
        .map(|files| files.to_vec())
        .unwrap_or_default();

    let total_files = 1 + extra_files.len();

    // Скачиваем основной файл
    let main_filename = url.rsplit('/').next().unwrap_or("model");
    let main_dest = dest_path.join(main_filename);
    download_file(&url, &main_dest, &model_id, 0, total_files, &app).await?;

    // Скачиваем дополнительные файлы
    for (idx, (extra_url, rel_path)) in extra_files.iter().enumerate() {
        let extra_dest = dest_path.join(rel_path);
        download_file(extra_url, &extra_dest, &model_id, idx + 1, total_files, &app).await?;
    }

    let _ = app.emit("download://complete", serde_json::json!({
        "model_id": model_id,
    }));

    tracing::info!("Модель '{}' скачана ({} файлов)", model_id, total_files);
    Ok(())
}

/// Tauri-команда: определить, работает ли Wayland
#[tauri::command]
fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
}

/// Конвертация формата хоткея из Tauri ("Super+Alt+Control+Space") в gsettings ("<Super><Alt><Control>space")
fn tauri_hotkey_to_gsettings(hotkey: &str) -> String {
    if hotkey.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = hotkey.split('+').collect();
    let mut mods = String::new();
    let mut key = String::new();
    for part in &parts {
        match *part {
            "Super" => mods.push_str("<Super>"),
            "Alt" => mods.push_str("<Alt>"),
            "Control" => mods.push_str("<Control>"),
            "Shift" => mods.push_str("<Shift>"),
            "`" => key = "grave".to_string(),
            k => key = k.to_lowercase(),
        }
    }
    format!("{}{}", mods, key)
}

/// Маппинг латинских клавиш → XKB keysym кириллических (для GNOME gsettings)
fn latin_to_cyrillic_keysym(key: &str) -> Option<&'static str> {
    match key {
        "q" => Some("Cyrillic_shorti"),    // й
        "w" => Some("Cyrillic_tse"),       // ц
        "e" => Some("Cyrillic_u"),         // у
        "r" => Some("Cyrillic_ka"),        // к
        "t" => Some("Cyrillic_ie"),        // е
        "y" => Some("Cyrillic_en"),        // н
        "u" => Some("Cyrillic_ghe"),       // г
        "i" => Some("Cyrillic_sha"),       // ш
        "o" => Some("Cyrillic_shcha"),     // щ
        "p" => Some("Cyrillic_ze"),        // з
        "a" => Some("Cyrillic_ef"),        // ф
        "s" => Some("Cyrillic_yeru"),      // ы
        "d" => Some("Cyrillic_ve"),        // в
        "f" => Some("Cyrillic_a"),         // а
        "g" => Some("Cyrillic_pe"),        // п
        "h" => Some("Cyrillic_er"),        // р
        "j" => Some("Cyrillic_o"),         // о
        "k" => Some("Cyrillic_el"),        // л
        "l" => Some("Cyrillic_de"),        // д
        "z" => Some("Cyrillic_ya"),        // я
        "x" => Some("Cyrillic_che"),       // ч
        "c" => Some("Cyrillic_es"),        // с
        "v" => Some("Cyrillic_em"),        // м
        "b" => Some("Cyrillic_i"),         // и
        "n" => Some("Cyrillic_te"),        // т
        "m" => Some("Cyrillic_softsign"),  // ь
        _ => None,
    }
}

/// Tauri-команда: проверить, занята ли комбинация клавиш в GNOME
#[tauri::command]
fn check_hotkey_conflict(hotkey: String) -> Result<Option<String>, String> {
    if hotkey.is_empty() {
        return Ok(None);
    }
    let binding = tauri_hotkey_to_gsettings(&hotkey);
    if binding.is_empty() {
        return Ok(None);
    }

    // Сканируем все схемы GNOME на совпадение
    let schemas = [
        "org.gnome.desktop.wm.keybindings",
        "org.gnome.shell.keybindings",
        "org.gnome.mutter.keybindings",
    ];

    for schema in &schemas {
        let output = std::process::Command::new("gsettings")
            .args(["list-recursively", schema])
            .output();
        if let Ok(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if line.contains(&binding) {
                    // Извлекаем имя настройки (второе слово в строке)
                    let name = line.split_whitespace().nth(1).unwrap_or("???");
                    return Ok(Some(format!("{} ({})", name, schema)));
                }
            }
        }
    }

    // Проверяем custom keybindings (кроме наших arcanaglyph-*)
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.settings-daemon.plugins.media-keys", "custom-keybindings"])
        .output()
        .map_err(|e| e.to_string())?;
    let paths_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if paths_str != "@as []" && !paths_str.is_empty() {
        let paths: Vec<String> = paths_str.trim_matches(|c| c == '[' || c == ']')
            .split(',')
            .map(|s| s.trim().trim_matches('\'').trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let base = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
        for path in &paths {
            // Пропускаем наши собственные слоты
            if path.contains("arcanaglyph-") {
                continue;
            }
            let schema_path = format!("{}:{}", base, path);
            let out = std::process::Command::new("gsettings")
                .args(["get", &schema_path, "binding"])
                .output();
            if let Ok(out) = out {
                let existing = String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_string();
                if existing == binding {
                    let name_out = std::process::Command::new("gsettings")
                        .args(["get", &schema_path, "name"])
                        .output();
                    let name = name_out.map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_string())
                        .unwrap_or_else(|_| "???".to_string());
                    return Ok(Some(format!("{} (custom keybinding)", name)));
                }
            }
        }
    }

    Ok(None)
}

/// Tauri-команда: зарегистрировать глобальные хоткеи через gsettings (Wayland/GNOME)
#[tauri::command]
fn register_gnome_hotkeys(hotkey_trigger: String, hotkey_pause: String) -> Result<(), String> {
    // Получаем текущий список custom keybindings
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.settings-daemon.plugins.media-keys", "custom-keybindings"])
        .output()
        .map_err(|e| format!("Не удалось вызвать gsettings: {}", e))?;
    let current = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Определяем слоты для ArcanaGlyph (ищем существующие или берём свободные)
    let ag_trigger_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-trigger/";
    let ag_trigger_cyr_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-trigger-cyr/";
    let ag_pause_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-pause/";
    let ag_pause_cyr_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-pause-cyr/";
    let base = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";

    // Вспомогательная функция для выполнения gsettings set
    let gs_set = |path: &str, key: &str, val: &str| -> Result<(), String> {
        let schema_path = format!("{}:{}", base, path);
        std::process::Command::new("gsettings")
            .args(["set", &schema_path, key, val])
            .output()
            .map_err(|e| format!("gsettings set {} {}: {}", key, val, e))?;
        Ok(())
    };

    // Определяем путь к скриптам
    let scripts_dir = CoreConfig::scripts_dir()
        .ok_or_else(|| "Не удалось определить директорию скриптов".to_string())?;
    let trigger_cmd = scripts_dir.join("ag-trigger").display().to_string();
    let pause_cmd = scripts_dir.join("ag-pause").display().to_string();

    // Регистрируем trigger (латиница)
    if !hotkey_trigger.is_empty() {
        let binding = tauri_hotkey_to_gsettings(&hotkey_trigger);
        gs_set(ag_trigger_path, "name", "'ArcanaGlyph Trigger'")?;
        gs_set(ag_trigger_path, "command", &format!("'{}'", trigger_cmd))?;
        gs_set(ag_trigger_path, "binding", &format!("'{}'", binding))?;

        // Кириллический дубль (для русской раскладки)
        let key = binding.rsplit('>').next().unwrap_or("");
        if let Some(cyrillic) = latin_to_cyrillic_keysym(key) {
            let mods = &binding[..binding.len() - key.len()];
            let cyr_binding = format!("{}{}", mods, cyrillic);
            gs_set(ag_trigger_cyr_path, "name", "'ArcanaGlyph Trigger (RU)'")?;
            gs_set(ag_trigger_cyr_path, "command", &format!("'{}'", trigger_cmd))?;
            gs_set(ag_trigger_cyr_path, "binding", &format!("'{}'", cyr_binding))?;
        }
    }

    // Регистрируем pause (латиница)
    if !hotkey_pause.is_empty() {
        let binding = tauri_hotkey_to_gsettings(&hotkey_pause);
        gs_set(ag_pause_path, "name", "'ArcanaGlyph Pause'")?;
        gs_set(ag_pause_path, "command", &format!("'{}'", pause_cmd))?;
        gs_set(ag_pause_path, "binding", &format!("'{}'", binding))?;

        // Кириллический дубль
        let key = binding.rsplit('>').next().unwrap_or("");
        if let Some(cyrillic) = latin_to_cyrillic_keysym(key) {
            let mods = &binding[..binding.len() - key.len()];
            let cyr_binding = format!("{}{}", mods, cyrillic);
            gs_set(ag_pause_cyr_path, "name", "'ArcanaGlyph Pause (RU)'")?;
            gs_set(ag_pause_cyr_path, "command", &format!("'{}'", pause_cmd))?;
            gs_set(ag_pause_cyr_path, "binding", &format!("'{}'", cyr_binding))?;
        }
    }

    // Обновляем список custom-keybindings — добавляем наши пути если их нет
    let mut paths: Vec<String> = if current == "@as []" || current.is_empty() {
        vec![]
    } else {
        // Парсим ['path1', 'path2', ...]
        current.trim_matches(|c| c == '[' || c == ']')
            .split(',')
            .map(|s| s.trim().trim_matches('\'').trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    if !hotkey_trigger.is_empty() && !paths.iter().any(|p| p == ag_trigger_path) {
        paths.push(ag_trigger_path.to_string());
    }
    // Кириллический дубль trigger
    let trigger_binding = tauri_hotkey_to_gsettings(&hotkey_trigger);
    let trigger_key = trigger_binding.rsplit('>').next().unwrap_or("");
    if !hotkey_trigger.is_empty() && latin_to_cyrillic_keysym(trigger_key).is_some()
        && !paths.iter().any(|p| p == ag_trigger_cyr_path)
    {
        paths.push(ag_trigger_cyr_path.to_string());
    }
    if !hotkey_pause.is_empty() && !paths.iter().any(|p| p == ag_pause_path) {
        paths.push(ag_pause_path.to_string());
    }
    // Кириллический дубль pause
    let pause_binding = tauri_hotkey_to_gsettings(&hotkey_pause);
    let pause_key = pause_binding.rsplit('>').next().unwrap_or("");
    if !hotkey_pause.is_empty() && latin_to_cyrillic_keysym(pause_key).is_some()
        && !paths.iter().any(|p| p == ag_pause_cyr_path)
    {
        paths.push(ag_pause_cyr_path.to_string());
    }

    let paths_str = format!("[{}]", paths.iter().map(|p| format!("'{}'", p)).collect::<Vec<_>>().join(", "));
    std::process::Command::new("gsettings")
        .args(["set", "org.gnome.settings-daemon.plugins.media-keys", "custom-keybindings", &paths_str])
        .output()
        .map_err(|e| format!("Не удалось обновить список keybindings: {}", e))?;

    tracing::info!("GNOME хоткеи зарегистрированы: trigger='{}', pause='{}'", hotkey_trigger, hotkey_pause);
    Ok(())
}

/// Проверяет, установлена ли модель (не просто существование директории, а ключевые файлы)
fn is_model_installed(path: &std::path::Path, transcriber_type: &str) -> bool {
    if !path.exists() {
        return false;
    }
    match transcriber_type {
        "gigaam" => path.join("v3_e2e_ctc.int8.onnx").exists() && path.join("v3_e2e_ctc_vocab.txt").exists(),
        "qwen3asr" => path.join("onnx_models/encoder_conv.onnx").exists() && path.join("tokenizer.json").exists(),
        "vosk" => path.join("conf").exists() || path.join("am").exists() || path.join("graph").exists(),
        _ => path.exists(),
    }
}

/// Tauri-команда: получить реестр моделей с проверкой наличия файлов
#[tauri::command]
fn get_models() -> Result<serde_json::Value, String> {
    let config = CoreConfig::load().map_err(|e| e.to_string())?;
    let models = arcanaglyph_core::transcription_models::all();
    let result: Vec<_> = models.iter().map(|m| {
        let path = match m.transcriber_type {
            "vosk" => &config.model_path,
            "whisper" => &config.whisper_model_path,
            "gigaam" => &config.gigaam_model_path,
            "qwen3asr" => &config.qwen3asr_model_path,
            _ => &config.model_path,
        };
        serde_json::json!({
            "id": m.id,
            "display_name": m.display_name,
            "transcriber_type": m.transcriber_type,
            "default_filename": m.default_filename,
            "description": m.description,
            "size": m.size,
            "download_url": m.download_url,
            "installed": is_model_installed(path, m.transcriber_type),
        })
    }).collect();
    Ok(serde_json::json!(result))
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

    // Управляем автозапуском
    set_autostart(config.autostart);

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
    use arcanaglyph_core::gigaam::transcriber::GigaAmTranscriber;
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
        "gigaam" => {
            let name = config.gigaam_model_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "gigaam".to_string());
            (name, "gigaam".to_string())
        }
        "qwen3asr" => {
            let name = config.qwen3asr_model_path.file_name()
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
        "gigaam" => {
            let t = GigaAmTranscriber::new(&config.gigaam_model_path).map_err(|e| e.to_string())?;
            (Box::new(t), config.sample_rate)
        }
        "qwen3asr" => {
            let t = arcanaglyph_core::qwen3asr::transcriber::Qwen3AsrTranscriber::new(&config.qwen3asr_model_path).map_err(|e| e.to_string())?;
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
    let hotkey_pause = config.hotkey_pause.clone();

    // Строки хоткеев для сравнения в handler
    let trigger_hk = Arc::new(hotkey.clone());
    let pause_hk = Arc::new(hotkey_pause.clone());

    tauri::Builder::default()
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
                                tracing::info!("Горячая клавиша паузы: {}", sc_str);
                                engine.pause();
                            } else if sc_str == *trigger_hk.as_ref() {
                                tracing::info!("Горячая клавиша триггера: {}", sc_str);
                                engine.trigger();
                            }
                        }
                    }
                })
                .build(),
        )
        .setup(move |app| {
            // Устанавливаем UDP-скрипты для Wayland (если ещё не установлены)
            // Создаём директорию моделей если не существует
            if let Some(models_dir) = CoreConfig::models_dir() {
                let _ = std::fs::create_dir_all(&models_dir);

                // Автоскачивание GigaAM v3 при первом запуске
                let gigaam_dir = models_dir.join("gigaam-v3-e2e-ctc");
                if !gigaam_dir.join("v3_e2e_ctc.int8.onnx").exists() {
                    tracing::info!("GigaAM v3 не найдена — скачиваю автоматически...");
                    let _ = std::fs::create_dir_all(&gigaam_dir);
                    let files = [
                        ("https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc.int8.onnx", "v3_e2e_ctc.int8.onnx"),
                        ("https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc_vocab.txt", "v3_e2e_ctc_vocab.txt"),
                    ];
                    let gd = gigaam_dir.clone();
                    tauri::async_runtime::spawn(async move {
                        for (url, filename) in &files {
                            let dest = gd.join(filename);
                            tracing::info!("Скачиваю {} ...", filename);
                            match reqwest::get(*url).await {
                                Ok(response) => {
                                    match response.bytes().await {
                                        Ok(bytes) => {
                                            if let Err(e) = tokio::fs::write(&dest, &bytes).await {
                                                tracing::error!("Ошибка записи {}: {}", filename, e);
                                            } else {
                                                tracing::info!("{} скачан ({}MB)", filename, bytes.len() / 1_000_000);
                                            }
                                        }
                                        Err(e) => tracing::error!("Ошибка скачивания {}: {}", filename, e),
                                    }
                                }
                                Err(e) => tracing::error!("Ошибка запроса {}: {}", filename, e),
                            }
                        }
                    });
                }
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
                    let hours = CoreConfig::load()
                        .map(|c| c.retention_hours)
                        .unwrap_or(0);
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
            let start_minimized = CoreConfig::load()
                .map(|c| c.start_minimized)
                .unwrap_or(false);

            let window_visible = Arc::new(AtomicBool::new(!start_minimized));
            app.manage(window_visible.clone());

            // Если запуск в свёрнутом виде — скрываем окно сразу
            if start_minimized
                && let Some(window) = app.get_webview_window("main")
            {
                let _ = window.hide();
            }

            // Engine создаётся в фоне — окно показывается сразу (если не minimized)
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
                                                let _ = app_h.emit("engine://model-preloaded", serde_json::json!({"model": name}));
                                            }
                                            Err(err) => tracing::warn!("Не удалось предзагрузить модель: {}", err),
                                        }
                                    }
                                });
                            }
                        }

                        // Event loop: пробрасываем события engine → фронтенд
                        let app_handle_events = app_handle_load.clone();
                        let engine_state_events = engine_state_load.clone();
                        tokio::spawn(async move {
                            loop {
                                match rx.recv().await {
                                    Ok(event) => {
                                        match &event {
                                            EngineEvent::RecordingStarted | EngineEvent::RecordingResumed => {
                                                tray::set_tray_text(&app_handle_events, "Остановить запись");
                                                tray::set_tray_recording(&app_handle_events, true);
                                                // Показываем виджет записи (если включён в настройках)
                                                if engine_state_events.get().is_some_and(|e| e.show_widget())
                                                    && let Some(w) = app_handle_events.get_webview_window("widget")
                                                {
                                                    let _ = w.show();
                                                    let _ = w.set_focus();
                                                }
                                            }
                                            EngineEvent::RecordingPaused => {
                                                tray::set_tray_text(&app_handle_events, "Продолжить запись");
                                                tray::set_tray_state(&app_handle_events, tray::TrayState::Paused);
                                                // Виджет остаётся видимым при паузе
                                            }
                                            EngineEvent::Transcribing => {
                                                tray::set_tray_text(&app_handle_events, "Транскрибация...");
                                                tray::set_tray_recording(&app_handle_events, false);
                                                // Скрываем виджет — запись окончена
                                                if let Some(w) = app_handle_events.get_webview_window("widget") {
                                                    let _ = w.hide();
                                                }
                                            }
                                            EngineEvent::FinishedProcessing => {
                                                tray::set_tray_text(&app_handle_events, "Начать запись");
                                                tray::set_tray_recording(&app_handle_events, false);
                                                // Скрываем виджет (страховка)
                                                if let Some(w) = app_handle_events.get_webview_window("widget") {
                                                    let _ = w.hide();
                                                }
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

            // Создаём виджет записи программно (для точного контроля размера)
            {
                let widget_width = 220.0;
                let widget_height = 40.0;
                let mut builder = tauri::WebviewWindowBuilder::new(
                    app,
                    "widget",
                    tauri::WebviewUrl::App("widget.html".into()),
                )
                .title("")
                .inner_size(widget_width, widget_height)
                .resizable(false)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .visible(false)
                .skip_taskbar(true);

                // Позиционируем в правом верхнем углу экрана
                if let Some(monitor) = app.primary_monitor().ok().flatten() {
                    let screen = monitor.size();
                    let scale = monitor.scale_factor();
                    let x = (screen.width as f64 / scale) - widget_width - 24.0;
                    builder = builder.position(x, 48.0);
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

            // Авторегистрация горячих клавиш в GNOME (Wayland) при первом запуске
            {
                let is_wayland = std::env::var("XDG_SESSION_TYPE")
                    .map(|v| v == "wayland")
                    .unwrap_or(false);
                if is_wayland && !hotkey.is_empty() {
                    // Проверяем, зарегистрированы ли уже наши хоткеи
                    let check = std::process::Command::new("gsettings")
                        .args(["get", "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-trigger/", "binding"])
                        .output();
                    let needs_register = match check {
                        Ok(out) => {
                            let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
                            val.is_empty() || val == "''" || val.contains("No such")
                        }
                        Err(_) => true,
                    };
                    if needs_register {
                        tracing::info!("Первый запуск на Wayland — регистрирую горячие клавиши в GNOME...");
                        let _ = register_gnome_hotkeys(hotkey.clone(), hotkey_pause.clone());
                    }
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
        .invoke_handler(tauri::generate_handler![trigger, pause, get_audio_level, is_recording, is_paused, is_model_loaded, get_loaded_models, get_models, download_model, is_wayland, check_hotkey_conflict, register_gnome_hotkeys, hide_window, load_config, save_config, get_history, delete_history_entry, clear_history, retranscribe, get_audio_data])
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
