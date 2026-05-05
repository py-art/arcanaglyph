// crates/arcanaglyph-app/src/main.rs

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tray;

use arcanaglyph_core::config::TranscriberType;
use arcanaglyph_core::history::HistoryDB;
use arcanaglyph_core::{ArcanaEngine, CoreConfig, EngineEvent};
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

/// Tauri-команда: отменить текущую транскрибацию (только Whisper). Возвращает
/// `true` если активный движок поддерживает cancel и сигнал отправлен; `false`
/// если нет (Vosk / GigaAM / Qwen3-ASR — там нет API для прерывания инференса).
#[tauri::command]
async fn cancel_transcription(engine: tauri::State<'_, EngineState>) -> Result<bool, String> {
    Ok(get_engine(&engine)?.cancel_transcription())
}

/// Tauri-команда: поддерживает ли активный движок отмену. UI использует это,
/// чтобы показывать / скрывать кнопку «Стоп» в transcribing-состоянии.
#[tauri::command]
async fn active_supports_cancel(engine: tauri::State<'_, EngineState>) -> Result<bool, String> {
    Ok(get_engine(&engine)?.active_supports_cancel())
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

/// Tauri-команда: имя текущего активного default-микрофона (через cpal).
/// Фронтенд использует для отображения "Активный микрофон: ..." в Settings и
/// чтобы записать gain под правильный device-key в `mic_gain_per_device`.
/// Возвращает пустую строку если устройства нет.
#[tauri::command]
fn get_default_input_device_name() -> String {
    arcanaglyph_core::audio::default_input_device_name().unwrap_or_default()
}

/// Tauri-команда: список движков, включённых в текущую сборку (по cargo features).
/// Фронтенд использует это, чтобы пометить недоступные пункты в dropdown'е как disabled.
#[tauri::command]
fn get_compiled_engines() -> Vec<&'static str> {
    TranscriberType::compiled_engines()
        .into_iter()
        .map(|t| t.as_str())
        .collect()
}

/// Tauri-команда: какие SIMD-фичи доступны на текущем CPU. Фронтенд использует это,
/// чтобы предупредить пользователя при выборе тяжёлой модели (Whisper Large без AVX2
/// — это 10-30× замедление).
#[tauri::command]
fn get_cpu_features() -> serde_json::Value {
    #[cfg(target_arch = "x86_64")]
    {
        serde_json::json!({
            "avx": std::is_x86_feature_detected!("avx"),
            "avx2": std::is_x86_feature_detected!("avx2"),
            "fma": std::is_x86_feature_detected!("fma"),
        })
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        serde_json::json!({
            "avx": false,
            "avx2": false,
            "fma": false,
        })
    }
}

/// Управляет автозапуском через .desktop файл в ~/.config/autostart/
#[cfg(target_os = "linux")]
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

// Заглушка автозапуска для Windows/macOS.
// На Windows нужен HKCU\Software\Microsoft\Windows\CurrentVersion\Run,
// на macOS — ~/Library/LaunchAgents/*.plist. Оставлено на следующий этап портирования.
#[cfg(not(target_os = "linux"))]
fn set_autostart(_enabled: bool) {}

/// Устанавливает UDP-скрипты ag-trigger и ag-pause (для Wayland)
#[cfg(target_os = "linux")]
fn install_wayland_scripts() {
    // Скрипты ставим на ЛЮБОМ Linux: на Wayland tauri-plugin-global-shortcut
    // вообще не работает (нет X11 grab), на X11+GNOME он часто не доставляет
    // event'ы (mutter перехватывает раньше). В обоих случаях нативные GNOME
    // custom-keybindings → ag-trigger → UDP — единственное что работает надёжно.
    let bin_dir = match CoreConfig::scripts_dir() {
        Some(d) => d,
        None => return,
    };
    let _ = std::fs::create_dir_all(&bin_dir);

    let scripts = [
        (
            "ag-trigger",
            "#!/bin/bash\n# ArcanaGlyph: UDP-триггер записи\necho \"trigger\" | /usr/bin/nc -u -w0 127.0.0.1 9002\n",
        ),
        (
            "ag-pause",
            "#!/bin/bash\n# ArcanaGlyph: UDP-триггер паузы\necho \"pause\" | /usr/bin/nc -u -w0 127.0.0.1 9002\n",
        ),
    ];

    for (name, content) in &scripts {
        let path = bin_dir.join(name);
        if !path.exists() {
            if let Err(e) = std::fs::write(&path, content) {
                tracing::warn!("Не удалось создать {}: {}", path.display(), e);
                continue;
            }
            // chmod +x
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
            tracing::info!("Установлен скрипт: {}", path.display());
        }
    }
}

// Заглушка установки Wayland-скриптов для Windows/macOS — там нет ни Wayland,
// ни /usr/bin/nc; UDP-триггер всё ещё доступен через прямую отправку датаграмм.
#[cfg(not(target_os = "linux"))]
fn install_wayland_scripts() {}

/// Скачивание одного файла с прогрессом.
///
/// Атомарность: пишем в `<dest>.partial` и переименовываем по успеху. Если процесс
/// прервётся (Ctrl+C, обрыв сети, нехватка места) — целевой файл не появится,
/// и следующий запуск перекачает с нуля. Это лечит «фантомно установленные» модели,
/// которые потом валятся в whisper_model_load с «expected N tensors, got 5».
///
/// `min_size` — необязательный минимальный размер в байтах. Если задан и фактический
/// размер меньше — возвращаем ошибку, файл `.partial` остаётся (перекачается заново).
async fn download_file(
    url: &str,
    dest: &std::path::Path,
    model_id: &str,
    file_idx: usize,
    total_files: usize,
    min_size: Option<u64>,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    use futures_util::StreamExt;
    use tauri::Emitter;

    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let filename = dest.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    // Скачиваем во временный *.partial, по успеху делаем atomic rename.
    let partial = {
        let mut s = dest.as_os_str().to_os_string();
        s.push(".partial");
        std::path::PathBuf::from(s)
    };
    tracing::info!(
        "Скачивание [{}/{}] {} → {}",
        file_idx + 1,
        total_files,
        filename,
        dest.display()
    );

    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("Ошибка запроса {}: {}", filename, e))?;
    let total_size = response.content_length().unwrap_or(0);

    let mut stream = response.bytes_stream();
    let mut file = tokio::fs::File::create(&partial)
        .await
        .map_err(|e| format!("Не удалось создать {}: {}", partial.display(), e))?;

    let mut downloaded: u64 = 0;
    let mut last_progress: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Ошибка скачивания {}: {}", filename, e))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| format!("Ошибка записи {}: {}", filename, e))?;

        downloaded += chunk.len() as u64;
        let progress_pct = (downloaded * 100).checked_div(total_size).unwrap_or(0);
        if progress_pct != last_progress {
            last_progress = progress_pct;
            let _ = app.emit(
                "download://progress",
                serde_json::json!({
                    "model_id": model_id,
                    "file": filename,
                    "file_idx": file_idx,
                    "total_files": total_files,
                    "downloaded": downloaded,
                    "total": total_size,
                    "percent": progress_pct,
                }),
            );
        }
    }
    // Гарантируем, что данные сброшены на диск перед rename
    if let Err(e) = tokio::io::AsyncWriteExt::flush(&mut file).await {
        return Err(format!("Ошибка flush {}: {}", filename, e));
    }
    drop(file);

    // Валидация размера до rename — иначе мы рискуем «успешно» переименовать обрезанный файл.
    if let Some(min) = min_size
        && downloaded < min
    {
        return Err(format!(
            "Скачано {} байт для {}, ожидалось не меньше {} (источник вернул обрезанный ответ)",
            downloaded, filename, min
        ));
    }

    tokio::fs::rename(&partial, dest).await.map_err(|e| {
        format!(
            "Не удалось переименовать {} → {}: {}",
            partial.display(),
            dest.display(),
            e
        )
    })?;
    Ok(())
}

/// Распаковывает `.zip`, рассчитывая что архив имеет top-level директорию с именем,
/// совпадающим с `expected_dir` (стандарт alphacephei для Vosk-моделей:
/// `vosk-model-ru-0.42.zip` → `vosk-model-ru-0.42/{am,conf,graph,...}`).
///
/// Распаковка идёт в **родителя `expected_dir`**, чтобы top-level dir архива создал
/// сам `expected_dir`. ВНИМАНИЕ: `zip_path.parent()` для нашей раскладки совпадает с
/// `expected_dir` (zip лежит ВНУТРИ него), так что extract'ить туда нельзя — получится
/// вложенность `expected_dir/expected_dir_name/...` и sanity-check не пройдёт.
///
/// После успешной распаковки сам `.zip` удаляется (~1.8 ГБ места).
///
/// Эмитит `download://extracting` сразу после старта чтобы UI заменил текст с
/// «Скачивание…» на «Распаковка…» — фронтенд иначе видит застывший 100% прогресс
/// на 30-90с (на N5095).
///
/// При ошибке распаковки **архив не удаляется** — пользователю нет смысла перекачивать
/// 1.8 ГБ если zip целый, проблема в чём-то другом (диск, права, нестандартная структура).
/// Удаляются только частично распакованные артефакты (всё в `expected_dir` КРОМЕ самого
/// архива). На следующий клик «Скачать» вызывающий код пропустит download_file
/// (увидит валидный zip на диске) и сразу попробует распаковать заново.
async fn extract_zip_into_model_dir(
    zip_path: &std::path::Path,
    expected_dir: &std::path::Path,
    model_id: &str,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    use tauri::Emitter;

    let _ = app.emit("download://extracting", serde_json::json!({ "model_id": model_id }));

    // Распаковка идёт в РОДИТЕЛЯ expected_dir, не в родителя zip_path. Подробности — в
    // doc-комментарии выше.
    let extract_target = expected_dir
        .parent()
        .ok_or_else(|| format!("у {} нет родителя", expected_dir.display()))?
        .to_path_buf();
    let zip_path_owned = zip_path.to_path_buf();

    tracing::info!(
        "Распаковка {} → {} (ожидается top-level дир {} в архиве)",
        zip_path_owned.display(),
        extract_target.display(),
        expected_dir.file_name().and_then(|s| s.to_str()).unwrap_or("?")
    );

    // zip-крейт синхронный; распаковка ~1.8 ГБ → 2.6 ГБ на N5095 ~30-90с,
    // нельзя блокировать async runtime — поэтому через spawn_blocking.
    let extract_res = tokio::task::spawn_blocking(move || -> Result<(), String> {
        let f =
            std::fs::File::open(&zip_path_owned).map_err(|e| format!("открыть {}: {}", zip_path_owned.display(), e))?;
        let mut archive = zip::ZipArchive::new(f).map_err(|e| format!("zip read: {}", e))?;
        archive
            .extract(&extract_target)
            .map_err(|e| format!("zip extract: {}", e))
    })
    .await
    .map_err(|e| format!("spawn_blocking join: {}", e))?;

    if let Err(e) = extract_res {
        cleanup_extraction_artifacts(zip_path, expected_dir);
        return Err(e);
    }

    // Sanity-check: alphacephei Vosk-zip кладёт `conf/` рядом с `am/graph/...`.
    // Тот же чек используется в is_model_installed для vosk.
    if !expected_dir.join("conf").exists() {
        cleanup_extraction_artifacts(zip_path, expected_dir);
        return Err(format!(
            "после распаковки нет {}/conf — структура архива не та, что ожидалась",
            expected_dir.display()
        ));
    }

    if let Err(e) = std::fs::remove_file(zip_path) {
        tracing::warn!(
            "Не удалось удалить архив {} после распаковки: {}",
            zip_path.display(),
            e
        );
    }

    Ok(())
}

/// Удаляет содержимое `model_dir` КРОМЕ самого `zip_path` — после неудачной распаковки
/// частично созданные подпапки/файлы убираются, а архив сохраняется для retry без
/// повторной загрузки 1.8 ГБ. zip_path в нашей раскладке всегда лежит ВНУТРИ model_dir,
/// так что итерируем содержимое и пропускаем zip по имени файла.
fn cleanup_extraction_artifacts(zip_path: &std::path::Path, model_dir: &std::path::Path) {
    let zip_name = zip_path.file_name();
    let Ok(entries) = std::fs::read_dir(model_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if Some(entry.file_name().as_os_str()) == zip_name {
            continue; // не трогаем сам архив
        }
        let path = entry.path();
        let _ = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
    }
}

/// Проверяет наличие модели для активного транскрайбера и при необходимости
/// скачивает её через `download_file()`. Прогресс эмитится через `download://progress`,
/// после завершения — `download://complete`. Возвращает Ok(()) когда модель установлена
/// (включая случай, когда она уже была на диске).
///
/// Защита от прерванного скачивания: если файл существует, но меньше
/// `model.expected_min_size_bytes` — он считается повреждённым и удаляется
/// перед перекачиванием. Это лечит ситуацию, когда `make run` был оборван и
/// `whisper_model_load` падает с «expected N tensors, got 5».
async fn ensure_active_model(transcriber_type: &str, app: &tauri::AppHandle) -> Result<(), String> {
    use arcanaglyph_core::transcription_models;

    let cfg = CoreConfig::load().map_err(|e| e.to_string())?;
    let path = match transcriber_type {
        "vosk" => cfg.model_path.clone(),
        "whisper" => cfg.whisper_model_path.clone(),
        "gigaam" => cfg.gigaam_model_path.clone(),
        "qwen3asr" => cfg.qwen3asr_model_path.clone(),
        _ => return Ok(()), // unknown тип — пусть engine create вернёт нормальную ошибку
    };

    // Для Whisper в реестре несколько моделей (Tiny/Small/Large) с разными размерами.
    // Выбираем ту, чей `default_filename` совпадает с именем файла в `whisper_model_path` —
    // иначе валидация может ошибочно посчитать Tiny-файл (~75 МБ) повреждённым по порогу
    // Small (~400 МБ) или наоборот. Для остальных движков — одна модель на тип.
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let model = if transcriber_type == "whisper" && !filename.is_empty() {
        transcription_models::find_by_type_and_filename(transcriber_type, filename)
    } else {
        transcription_models::find_by_transcriber_type(transcriber_type)
    };
    let min_size = model.and_then(|m| m.expected_min_size_bytes);

    if is_model_installed(&path, transcriber_type, min_size) {
        return Ok(()); // модель уже на месте и валидна
    }

    let model = model.ok_or_else(|| format!("В реестре нет модели для движка '{}'", transcriber_type))?;
    tracing::info!(
        "Модель для '{}' не найдена локально или повреждена — скачиваю '{}' ({})",
        transcriber_type,
        model.display_name,
        model.size
    );

    // Whisper: путь — это файл (whisper_model_path указывает прямо на .bin).
    // Остальные движки: путь — это директория с файлами модели; имя главного файла
    // берём из последнего сегмента URL (а не из `default_filename` — это имя ДИРЕКТОРИИ,
    // не файла; смешение сломало бы сохранение скачанной модели).
    let main_dest: std::path::PathBuf = if transcriber_type == "whisper" {
        path.clone()
    } else {
        let _ = std::fs::create_dir_all(&path);
        let main_filename = model.download_url.rsplit('/').next().unwrap_or("model.bin");
        path.join(main_filename)
    };

    // Чистим повреждённый главный файл (меньше порога), чтобы не путать atomic-rename.
    if let (Some(min), Ok(meta)) = (min_size, std::fs::metadata(&main_dest))
        && meta.is_file()
        && meta.len() < min
    {
        tracing::warn!(
            "Файл модели повреждён (размер {} байт, минимум {}) — будет перекачан: {}",
            meta.len(),
            min,
            main_dest.display()
        );
        let _ = std::fs::remove_file(&main_dest);
    }

    let extras = model.extra_files.unwrap_or(&[]);
    let total_files = 1 + extras.len();
    // Тот же skip как в download_model: если zip уже скачан и валиден по размеру,
    // не перекачиваем 1.8 ГБ ради повторной попытки распаковки.
    let zip_already_downloaded = main_dest.extension().and_then(|s| s.to_str()) == Some("zip")
        && match (min_size, std::fs::metadata(&main_dest)) {
            (Some(min), Ok(m)) if m.is_file() => m.len() >= min,
            _ => false,
        };
    if zip_already_downloaded {
        tracing::info!(
            "zip-архив {} уже на диске и валидного размера — пропускаем загрузку",
            main_dest.display()
        );
    } else {
        download_file(model.download_url, &main_dest, model.id, 0, total_files, min_size, app).await?;
    }
    // Если основной файл — .zip-архив, распаковываем сразу после скачивания.
    // Сейчас касается только Vosk; для будущих архивных моделей сработает автоматом.
    if main_dest.extension().and_then(|s| s.to_str()) == Some("zip") {
        extract_zip_into_model_dir(&main_dest, &path, model.id, app).await?;
    }
    for (idx, (url, rel)) in extras.iter().enumerate() {
        let extra_dest = path.join(rel);
        // Для extra-файлов size-проверка не задаётся: размеры варьируются и порог в реестре пока один общий.
        download_file(url, &extra_dest, model.id, idx + 1, total_files, None, app).await?;
    }

    let _ = app.emit("download://complete", serde_json::json!({ "model_id": model.id }));
    tracing::info!("Модель '{}' успешно установлена", model.display_name);
    Ok(())
}

/// Tauri-команда: удалить файлы модели с диска + очистить совпадающий путь в config'е.
/// Если в config-поле для движка лежит ИМЕННО этот путь — оно затирается на пустой
/// (чтобы UI после re-render'а не показывал мёртвый путь, и engine при следующем
/// запуске на этом движке выбрасывал чистую ошибку «модель не выбрана» вместо
/// попытки прочитать несуществующий файл). Если у пользователя несколько моделей
/// одного движка (Whisper Tiny + Large) и удалена только одна — путь чистится
/// только если совпадает с удалённой.
///
/// Whisper: путь это `.bin` файл → `remove_file`.
/// Vosk / GigaAM / Qwen3-ASR: путь это директория → `remove_dir_all`.
#[tauri::command]
async fn delete_model(model_id: String, path: String) -> Result<(), String> {
    let model_info = arcanaglyph_core::transcription_models::find(&model_id)
        .ok_or_else(|| format!("Модель '{}' не найдена в реестре", model_id))?;
    let pb = std::path::PathBuf::from(&path);

    // Удаление файлов с диска (idempotent: если уже нет — продолжаем чистить config)
    if pb.exists() {
        if model_info.transcriber_type == "whisper" {
            std::fs::remove_file(&pb).map_err(|e| format!("Не удалось удалить файл {}: {}", path, e))?;
            tracing::info!("Удалён файл модели: {}", path);
        } else if pb.is_dir() {
            std::fs::remove_dir_all(&pb).map_err(|e| format!("Не удалось удалить директорию {}: {}", path, e))?;
            tracing::info!("Удалена директория модели: {}", path);
        } else {
            std::fs::remove_file(&pb).map_err(|e| format!("Не удалось удалить {}: {}", path, e))?;
            tracing::info!("Удалён файл модели: {}", path);
        }
    }

    // Чистим config-путь, если он совпадает с удалённым.
    let mut config = CoreConfig::load().map_err(|e| e.to_string())?;
    let cleared = match model_info.transcriber_type {
        "vosk" if config.model_path == pb => {
            config.model_path = std::path::PathBuf::new();
            true
        }
        "whisper" if config.whisper_model_path == pb => {
            config.whisper_model_path = std::path::PathBuf::new();
            true
        }
        "gigaam" if config.gigaam_model_path == pb => {
            config.gigaam_model_path = std::path::PathBuf::new();
            true
        }
        "qwen3asr" if config.qwen3asr_model_path == pb => {
            config.qwen3asr_model_path = std::path::PathBuf::new();
            true
        }
        _ => false,
    };
    if cleared {
        config.save().map_err(|e| e.to_string())?;
        tracing::info!(
            "Очищен config-путь для движка '{}' после удаления модели",
            model_info.transcriber_type
        );
    }
    Ok(())
}

/// Tauri-команда: скачать модель (один или несколько файлов) с прогрессом
#[tauri::command]
async fn download_model(model_id: String, url: String, dest_dir: String, app: tauri::AppHandle) -> Result<(), String> {
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
    let min_size = model_info.and_then(|m| m.expected_min_size_bytes);

    // Guard: если модель уже установлена и валидна — не качаем заново. Это защищает от
    // двойного клика, ре-нажатия после ошибки в другой части flow и от ситуации
    // «config пустой, но файлы на диске лежат» (ручная подкладка). Config-путь обновим
    // ниже как обычно, чтобы карточка перерисовалась корректно.
    let already_installed = match model_info {
        Some(m) => {
            // Whisper хранит сам файл .bin как путь модели; остальные движки — директорию.
            let install_path: std::path::PathBuf = if m.transcriber_type == "whisper" {
                main_dest.clone()
            } else {
                dest_path.clone()
            };
            is_model_installed(&install_path, m.transcriber_type, m.expected_min_size_bytes)
        }
        None => false,
    };
    if already_installed {
        tracing::info!(
            "Модель '{}' уже установлена в {} — пропускаем скачивание",
            model_id,
            dest_path.display()
        );
    } else {
        // Если zip-архив уже скачан и не повреждён по размеру — пропускаем download_file,
        // сразу распаковываем. Это нужно когда прошлая попытка распаковки упала
        // (extract_zip_into_model_dir на ошибке оставляет zip нетронутым), и пользователь
        // повторно нажал «Скачать» — перекачивать 1.8 ГБ не нужно.
        let zip_already_downloaded = main_dest.extension().and_then(|s| s.to_str()) == Some("zip")
            && match (min_size, std::fs::metadata(&main_dest)) {
                (Some(min), Ok(m)) if m.is_file() => m.len() >= min,
                _ => false,
            };
        if zip_already_downloaded {
            tracing::info!(
                "zip-архив {} уже на диске и валидного размера — пропускаем загрузку, иду в распаковку",
                main_dest.display()
            );
        } else {
            download_file(&url, &main_dest, &model_id, 0, total_files, min_size, &app).await?;
        }

        // Если основной файл — .zip-архив, распаковываем сразу после скачивания.
        // Сейчас касается только Vosk; для будущих архивных моделей сработает автоматом.
        if main_dest.extension().and_then(|s| s.to_str()) == Some("zip") {
            extract_zip_into_model_dir(&main_dest, &dest_path, &model_id, &app).await?;
        }

        // Скачиваем дополнительные файлы (внутри else — если модель уже установлена,
        // is_model_installed проверил extras тоже, перекачивать их не нужно).
        for (idx, (extra_url, rel_path)) in extra_files.iter().enumerate() {
            let extra_dest = dest_path.join(rel_path);
            download_file(extra_url, &extra_dest, &model_id, idx + 1, total_files, None, &app).await?;
        }
    }

    // Авто-обновление config-пути для скачанной модели. Без этого:
    //   1. После delete_model config-путь пуст; пользователь нажимает «Скачать» — файл
    //      на диске есть, но config.whisper_model_path всё ещё "" → engine думает
    //      что модель не выбрана, dropdown в UI помечает её как «(нет модели)».
    //   2. Если пользователь хочет переключить варианты Whisper (Tiny/Large) через UI,
    //      достаточно нажать «Скачать» — путь в config'е обновится сам.
    // Whisper: путь — это сам .bin файл. Vosk/GigaAM/Qwen3-ASR — директория модели.
    if let Some(model_info) = model_info {
        let saved_path: std::path::PathBuf = if model_info.transcriber_type == "whisper" {
            main_dest.clone()
        } else {
            dest_path.clone()
        };
        if let Ok(mut config) = CoreConfig::load() {
            let updated = match model_info.transcriber_type {
                "vosk" => {
                    config.model_path = saved_path;
                    true
                }
                "whisper" => {
                    config.whisper_model_path = saved_path;
                    true
                }
                "gigaam" => {
                    config.gigaam_model_path = saved_path;
                    true
                }
                "qwen3asr" => {
                    config.qwen3asr_model_path = saved_path;
                    true
                }
                _ => false,
            };
            if updated {
                if let Err(e) = config.save() {
                    tracing::warn!("Не удалось сохранить config-путь после скачивания: {}", e);
                } else {
                    tracing::info!(
                        "Config-путь для движка '{}' обновлён на скачанный файл",
                        model_info.transcriber_type
                    );
                }
            }
        }
    }

    let _ = app.emit(
        "download://complete",
        serde_json::json!({
            "model_id": model_id,
        }),
    );

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
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
fn latin_to_cyrillic_keysym(key: &str) -> Option<&'static str> {
    match key {
        "q" => Some("Cyrillic_shorti"),   // й
        "w" => Some("Cyrillic_tse"),      // ц
        "e" => Some("Cyrillic_u"),        // у
        "r" => Some("Cyrillic_ka"),       // к
        "t" => Some("Cyrillic_ie"),       // е
        "y" => Some("Cyrillic_en"),       // н
        "u" => Some("Cyrillic_ghe"),      // г
        "i" => Some("Cyrillic_sha"),      // ш
        "o" => Some("Cyrillic_shcha"),    // щ
        "p" => Some("Cyrillic_ze"),       // з
        "a" => Some("Cyrillic_ef"),       // ф
        "s" => Some("Cyrillic_yeru"),     // ы
        "d" => Some("Cyrillic_ve"),       // в
        "f" => Some("Cyrillic_a"),        // а
        "g" => Some("Cyrillic_pe"),       // п
        "h" => Some("Cyrillic_er"),       // р
        "j" => Some("Cyrillic_o"),        // о
        "k" => Some("Cyrillic_el"),       // л
        "l" => Some("Cyrillic_de"),       // д
        "z" => Some("Cyrillic_ya"),       // я
        "x" => Some("Cyrillic_che"),      // ч
        "c" => Some("Cyrillic_es"),       // с
        "v" => Some("Cyrillic_em"),       // м
        "b" => Some("Cyrillic_i"),        // и
        "n" => Some("Cyrillic_te"),       // т
        "m" => Some("Cyrillic_softsign"), // ь
        // Спец-клавиша: на русской раскладке клавиша слева от 1 (`/~) даёт Ё.
        // Без этого маппинга keybinding `<Control>grave` не срабатывает на ru-раскладке —
        // GNOME ищет `<Control>Cyrillic_io` и не находит его.
        "grave" => Some("Cyrillic_io"), // Ё
        _ => None,
    }
}

/// Tauri-команда: проверить, занята ли комбинация клавиш в GNOME.
/// На Win/macOS возвращает None — там GNOME отсутствует.
#[tauri::command]
fn check_hotkey_conflict(hotkey: String) -> Result<Option<String>, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = hotkey;
        return Ok(None);
    }
    #[cfg(target_os = "linux")]
    check_hotkey_conflict_gnome(hotkey)
}

#[cfg(target_os = "linux")]
fn check_hotkey_conflict_gnome(hotkey: String) -> Result<Option<String>, String> {
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
        .args([
            "get",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    let paths_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if paths_str != "@as []" && !paths_str.is_empty() {
        let paths: Vec<String> = paths_str
            .trim_matches(|c| c == '[' || c == ']')
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
                let existing = String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .trim_matches('\'')
                    .to_string();
                if existing == binding {
                    let name_out = std::process::Command::new("gsettings")
                        .args(["get", &schema_path, "name"])
                        .output();
                    let name = name_out
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_string())
                        .unwrap_or_else(|_| "???".to_string());
                    return Ok(Some(format!("{} (custom keybinding)", name)));
                }
            }
        }
    }

    Ok(None)
}

/// Tauri-команда: зарегистрировать глобальные хоткеи через gsettings (Wayland/GNOME).
/// На Win/macOS — no-op.
#[tauri::command]
fn register_gnome_hotkeys(hotkey_trigger: String, hotkey_pause: String) -> Result<(), String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (hotkey_trigger, hotkey_pause);
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    register_gnome_hotkeys_linux(hotkey_trigger, hotkey_pause)
}

#[cfg(target_os = "linux")]
fn register_gnome_hotkeys_linux(hotkey_trigger: String, hotkey_pause: String) -> Result<(), String> {
    // Получаем текущий список custom keybindings
    let output = std::process::Command::new("gsettings")
        .args([
            "get",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
        ])
        .output()
        .map_err(|e| format!("Не удалось вызвать gsettings: {}", e))?;
    let current = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Определяем слоты для ArcanaGlyph (ищем существующие или берём свободные)
    let ag_trigger_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-trigger/";
    let ag_trigger_cyr_path =
        "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-trigger-cyr/";
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
    let scripts_dir =
        CoreConfig::scripts_dir().ok_or_else(|| "Не удалось определить директорию скриптов".to_string())?;
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
        current
            .trim_matches(|c| c == '[' || c == ']')
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
    if !hotkey_trigger.is_empty()
        && latin_to_cyrillic_keysym(trigger_key).is_some()
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
    if !hotkey_pause.is_empty()
        && latin_to_cyrillic_keysym(pause_key).is_some()
        && !paths.iter().any(|p| p == ag_pause_cyr_path)
    {
        paths.push(ag_pause_cyr_path.to_string());
    }

    let paths_str = format!(
        "[{}]",
        paths.iter().map(|p| format!("'{}'", p)).collect::<Vec<_>>().join(", ")
    );
    std::process::Command::new("gsettings")
        .args([
            "set",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
            &paths_str,
        ])
        .output()
        .map_err(|e| format!("Не удалось обновить список keybindings: {}", e))?;

    tracing::info!(
        "GNOME хоткеи зарегистрированы: trigger='{}', pause='{}'",
        hotkey_trigger,
        hotkey_pause
    );
    Ok(())
}

/// Проверяет, установлена ли модель (не просто существование директории, а ключевые файлы).
///
/// `min_size` — минимальный ожидаемый размер главного файла. Если задан и фактический
/// размер меньше — считаем, что файл повреждён (например, прерванное скачивание).
fn is_model_installed(path: &std::path::Path, transcriber_type: &str, min_size: Option<u64>) -> bool {
    if !path.exists() {
        return false;
    }

    // Главный файл, размер которого валидируем по `min_size` (если задан).
    // Для whisper это сам путь (это файл), для директорных движков — главный файл внутри.
    let main_file: Option<std::path::PathBuf> = match transcriber_type {
        "whisper" => Some(path.to_path_buf()),
        "gigaam" => Some(path.join("v3_e2e_ctc.int8.onnx")),
        "qwen3asr" => Some(path.join("tokenizer.json")),
        _ => None,
    };
    if let Some(ref f) = main_file {
        if !f.exists() {
            return false;
        }
        if let Some(min) = min_size {
            match std::fs::metadata(f) {
                Ok(meta) if meta.is_file() && meta.len() >= min => {}
                _ => return false,
            }
        }
    }

    // Дополнительные обязательные файлы (без проверки размера — только наличие)
    match transcriber_type {
        "gigaam" => path.join("v3_e2e_ctc_vocab.txt").exists(),
        "qwen3asr" => path.join("onnx_models/encoder_conv.onnx").exists(),
        "vosk" => path.join("conf").exists() || path.join("am").exists() || path.join("graph").exists(),
        _ => true,
    }
}

/// Tauri-команда: получить реестр моделей с проверкой наличия файлов.
/// Возвращает ВСЕ известные модели (включая недоступные в текущей сборке) с
/// флагом `available`, чтобы UI мог показать неактивные карточки в disabled-стиле.
///
/// Путь модели резолвится так:
///   1. Если в config'е под движок есть непустой путь — используем его.
///   2. Иначе fallback на `models_base_dir/<default_filename>` из реестра.
/// Это позволяет UI корректно отображать «Установлено» для моделей, чьи файлы
/// физически лежат в дефолтной директории, но config-поле для них пустое
/// (например после `delete_model` который чистит config-путь, или после
/// первой установки `.deb` где config пустой по умолчанию).
#[tauri::command]
fn get_models() -> Result<serde_json::Value, String> {
    let config = CoreConfig::load().map_err(|e| e.to_string())?;
    let models = arcanaglyph_core::transcription_models::all_with_availability();
    let result: Vec<_> = models
        .iter()
        .map(|(m, available)| {
            let config_path = match m.transcriber_type {
                "vosk" => &config.model_path,
                "whisper" => &config.whisper_model_path,
                "gigaam" => &config.gigaam_model_path,
                "qwen3asr" => &config.qwen3asr_model_path,
                _ => &config.model_path,
            };
            // Резолвим путь к ФАЙЛУ модели (для проверки наличия + UI display).
            //
            // Для Whisper в реестре две модели (Tiny + Large), но `whisper_model_path`
            // в config один. Если бы мы использовали config_path для обеих карточек,
            // одна из них показывала бы статус другой (например, Large card видела бы
            // `ggml-tiny.bin` если в config'е лежит он → размер не совпадает с
            // expected → installed=false → ложное «Не найдена», хотя файл Large
            // лежит в models_base_dir/ggml-large-v3-turbo.bin). Решение:
            // **для Whisper — всегда `models_base_dir/<default_filename>` per-model**,
            // независимо от config_path. Сам config_path всё ещё используется engine'ом
            // для загрузки активной модели через `getModelPathFromCard`/dropdown в UI.
            //
            // Для Vosk/GigaAM/Qwen3-ASR — одна модель на движок, конфликта нет.
            // Используем config_path; если пуст — fallback на default-локацию.
            let resolved_path: std::path::PathBuf = match m.transcriber_type {
                "whisper" => config.models_base_dir.join(m.default_filename),
                _ => {
                    if config_path.as_os_str().is_empty() {
                        config.models_base_dir.join(m.default_filename)
                    } else {
                        config_path.clone()
                    }
                }
            };
            // Файл «установлен» считаем только для доступных движков —
            // нет смысла показывать «зелёную галочку» для модели, которой backend нет.
            let installed =
                *available && is_model_installed(&resolved_path, m.transcriber_type, m.expected_min_size_bytes);
            // Path для UI — если файла нет на диске, отдаём пустую строку. Иначе
            // path input в карточке заполнялся бы дефолтной локацией даже после
            // удаления, что вводит в заблуждение («путь есть, файла нет»).
            let display_path = if installed {
                resolved_path.display().to_string()
            } else {
                String::new()
            };
            serde_json::json!({
                "id": m.id,
                "display_name": m.display_name,
                "transcriber_type": m.transcriber_type,
                "default_filename": m.default_filename,
                "description": m.description,
                "size": m.size,
                "download_url": m.download_url,
                "installed": installed,
                "available": *available,
                "path": display_path,
            })
        })
        .collect();
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
fn save_config(
    config: serde_json::Value,
    app: tauri::AppHandle,
    engine: tauri::State<'_, EngineState>,
) -> Result<(), String> {
    let config: CoreConfig = serde_json::from_value(config).map_err(|e| format!("Ошибка парсинга конфига: {}", e))?;
    config.save().map_err(|e| e.to_string())?;

    // Управляем автозапуском
    set_autostart(config.autostart);

    // Управляем видимостью трея
    tray::set_tray_visible(&app, config.show_tray);

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
fn set_history_filter(secs: u64) -> Result<(), String> {
    let mut cfg = CoreConfig::load().map_err(|e| e.to_string())?;
    cfg.history_filter_secs = secs;
    cfg.save().map_err(|e| e.to_string())
}

/// Tauri-команда: сохранить выбранный язык интерфейса (без применения к движку)
#[tauri::command]
fn set_language(lang: String) -> Result<(), String> {
    let mut cfg = CoreConfig::load().map_err(|e| e.to_string())?;
    cfg.language = lang;
    cfg.save().map_err(|e| e.to_string())
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

/// Tauri-команда: экспорт истории в файл (txt или csv)
#[tauri::command]
fn export_history(format: String, db: tauri::State<'_, Arc<HistoryDB>>) -> Result<String, String> {
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
async fn retranscribe(
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
fn get_audio_data(recording_id: i64, db: tauri::State<'_, Arc<HistoryDB>>) -> Result<serde_json::Value, String> {
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

/// Tauri-команда: скрыть окно в трей и обновить флаг видимости
#[tauri::command]
async fn hide_window(window: tauri::Window, visible: tauri::State<'_, Arc<AtomicBool>>) -> Result<(), String> {
    let _ = window.hide();
    visible.store(false, Ordering::Relaxed);
    Ok(())
}

/// Выбирает путь к `libonnxruntime.so` для load-dynamic backend ORT и записывает его в
/// `ORT_DYLIB_PATH`. ВАЖНО: вызывать ДО первого касания `ort` (первый вызов —
/// `Session::builder()` в `gigaam/transcriber.rs`). Не имеет эффекта если ORT_DYLIB_PATH
/// уже выставлена (например, Makefile при `make run`).
///
/// Приоритет:
/// 1. `ORT_DYLIB_PATH` в env — оставляем как есть (dev override).
/// 2. `/usr/local/lib/libonnxruntime.so` — self-build пользователя (десктоп с самосборкой ORT).
/// 3. Bundled в `.deb` — `/usr/lib/arcanaglyph/libonnxruntime-{avx2,noavx}.so`,
///    выбор по runtime AVX-detection.
///
/// Если ничего не нашли — оставляем env пустой и ort попробует системный dlopen
/// (LD_LIBRARY_PATH, /usr/lib, /etc/ld.so.cache). Это путь dev-сборки на машине без
/// нашего pre-arrangement'а — fallback логика ничего не ломает.
#[cfg(target_os = "linux")]
fn setup_ort_dylib_path() {
    use std::path::Path;

    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        tracing::info!(
            "ORT_DYLIB_PATH = {} (взят из env)",
            std::env::var("ORT_DYLIB_PATH").unwrap_or_default()
        );
        return;
    }

    let local_lib = Path::new("/usr/local/lib/libonnxruntime.so");
    if local_lib.exists() {
        // SAFETY: вызывается в main() до спавна тредов, до загрузки ort.
        unsafe { std::env::set_var("ORT_DYLIB_PATH", local_lib) };
        tracing::info!("ORT_DYLIB_PATH = {} (self-build override)", local_lib.display());
        return;
    }

    #[cfg(target_arch = "x86_64")]
    let bundled = if std::is_x86_feature_detected!("avx") {
        "/usr/lib/arcanaglyph/libonnxruntime-avx2.so"
    } else {
        "/usr/lib/arcanaglyph/libonnxruntime-noavx.so"
    };
    #[cfg(not(target_arch = "x86_64"))]
    let bundled = "/usr/lib/arcanaglyph/libonnxruntime.so";

    let bundled_path = Path::new(bundled);
    if bundled_path.exists() {
        // SAFETY: вызывается в main() до спавна тредов, до загрузки ort.
        unsafe { std::env::set_var("ORT_DYLIB_PATH", bundled_path) };
        tracing::info!("ORT_DYLIB_PATH = {} (bundled .deb)", bundled_path.display());
        return;
    }

    tracing::warn!(
        "ORT_DYLIB_PATH не выставлена и libonnxruntime.so не найдена ни в /usr/local/lib, \
         ни в /usr/lib/arcanaglyph. ORT попробует загрузить через системный dlopen — \
         если в LD_LIBRARY_PATH нет нужной либы, GigaAM/Qwen3-ASR упадут при инициализации."
    );
}

#[cfg(not(target_os = "linux"))]
fn setup_ort_dylib_path() {
    // На Windows/macOS ORT крейт ищет либу через системные механизмы — ничего не делаем.
}

/// Проставляет glib `g_prgname` в "arcanaglyph", чтобы GTK/GDK выставили `WM_CLASS`
/// в "arcanaglyph" вне зависимости от физического имени бинаря.
///
/// Зачем: в self-contained `.deb` у нас два бинаря — `arcanaglyph-avx` и
/// `arcanaglyph-noavx` (см. `assets/scripts/arcanaglyph-wrapper.sh`). По умолчанию
/// GTK берёт `g_prgname` из `argv[0]` → `WM_CLASS = "arcanaglyph-noavx"`. Это не
/// совпадает со `StartupWMClass=arcanaglyph` в `assets/arcanaglyph.desktop`, и
/// GNOME shell не привязывает работающее окно к ярлыку приложения — в Dash
/// появляется отдельная иконка с именем бинаря. После явной установки `g_prgname`
/// `WM_CLASS = "arcanaglyph"` и Dash корректно группирует окно с ярлыком.
///
/// Вызывать ДО любого GTK/GDK init (т.е. до `tauri::Builder::new()`).
#[cfg(target_os = "linux")]
fn setup_program_name() {
    unsafe extern "C" {
        fn g_set_prgname(prgname: *const std::ffi::c_char);
    }
    let name = std::ffi::CString::new("arcanaglyph").expect("static name without NULs");
    // SAFETY: glib `g_set_prgname` копирует строку в свой буфер; срок жизни
    // нашего CString не важен. Вызывается до спавна потоков и до GTK init.
    unsafe { g_set_prgname(name.as_ptr()) };
}

#[cfg(not(target_os = "linux"))]
fn setup_program_name() {}

fn main() {
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
    setup_ort_dylib_path();
    // Принудительно ставим WM_CLASS=arcanaglyph (для группировки в GNOME Dash).
    setup_program_name();

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
    // поэтому AVX не нужен. `gigaam-tract` — pure-Rust, AVX не требует.
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
            // Создаём директорию моделей если не существует. Сами модели качаем
            // позже через `ensure_active_model` — единым generic-механизмом для всех движков.
            if let Some(models_dir) = CoreConfig::models_dir() {
                let _ = std::fs::create_dir_all(&models_dir);
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

            // Скачивание модели и загрузка engine в фоне.
            // Выполняем СТРОГО последовательно: сначала убеждаемся, что модель активного
            // движка на диске (качаем при первом запуске), затем создаём engine. Это убирает
            // ERROR-логи при первом запуске («failed to open <model>») — engine видит файл.
            let app_handle_load = app.handle().clone();
            let engine_state_load = engine_state.clone();
            let engine_fallback_evt = engine_fallback.clone();
            let active_transcriber = config.transcriber.as_str().to_string();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = ensure_active_model(&active_transcriber, &app_handle_load).await {
                    tracing::error!(
                        "Не удалось подготовить модель для '{}': {}",
                        active_transcriber,
                        e
                    );
                    let _ = app_handle_load.emit(
                        "engine://error",
                        serde_json::json!({ "message": format!("Не удалось скачать модель: {}", e) }),
                    );
                    return;
                }

                let result = tokio::task::spawn_blocking(move || {
                    ArcanaEngine::new(config, window_visible)
                })
                .await;

                match result {
                    Ok(Ok(engine)) => {
                        // Подписываемся на события ПЕРЕД set, пока есть ownership
                        let mut rx = engine.subscribe();
                        let _ = engine_state_load.set(engine);
                        tracing::info!("Engine готов к работе");
                        let _ = app_handle_load.emit("engine://model-loaded", serde_json::json!({}));

                        // Если при старте сработал auto-fallback на дефолтный движок — сообщаем UI,
                        // чтобы тот показал toast «движок X недоступен, используется Y».
                        if let Some((original, fallback)) = engine_fallback_evt {
                            let _ = app_handle_load.emit(
                                "engine://fallback",
                                serde_json::json!({ "original": original, "fallback": fallback }),
                            );
                        }

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
                                            EngineEvent::ModelLoading(name) => ("engine://model-loading", serde_json::json!({"model": name})),
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

            // Скрываем иконку трея если выключена в настройках
            if !CoreConfig::load().map(|c| c.show_tray).unwrap_or(true) {
                tray::set_tray_visible(app.handle(), false);
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

            // Авторегистрация горячих клавиш в GNOME (Wayland И X11) при первом запуске.
            // Запускаем на любом GNOME-сеансе:
            // - на Wayland tauri-plugin-global-shortcut вообще не работает (нет X11);
            // - на X11+GNOME он часто не получает event'ы (mutter их перехватывает).
            // Нативные GNOME custom-keybindings → ag-trigger → UDP — единственный
            // надёжный путь для GNOME. Для не-GNOME DE (KDE/i3/sway) этот блок просто
            // тихо отвалится с ошибкой gsettings — там нужно настраивать вручную.
            #[cfg(target_os = "linux")]
            if !hotkey.is_empty() {
                // Проверяем, зарегистрированы ли уже наши хоткеи. Перерегистрируем если
                // отсутствует ЛЮБОЙ из четырёх slots — особенно cyr-варианты, которые
                // могли не быть созданы старой версией кода без mapping для grave.
                let probe = |slot: &str| -> bool {
                    let check = std::process::Command::new("gsettings")
                        .args([
                            "get",
                            &format!("org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/{}/", slot),
                            "binding",
                        ])
                        .output();
                    match check {
                        Ok(out) => {
                            let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
                            val.is_empty() || val == "''" || val.contains("No such")
                        }
                        Err(_) => true,
                    }
                };
                let needs_register = probe("arcanaglyph-trigger")
                    || probe("arcanaglyph-trigger-cyr")
                    || probe("arcanaglyph-pause")
                    || probe("arcanaglyph-pause-cyr");
                if needs_register {
                    tracing::info!("Регистрирую глобальные горячие клавиши в GNOME...");
                    if let Err(e) = register_gnome_hotkeys(hotkey.clone(), hotkey_pause.clone()) {
                        tracing::warn!("Не удалось зарегистрировать GNOME-хоткеи (не GNOME?): {}", e);
                    }
                }

                // Пинок XKB на X11+GNOME: заставляем mutter пере-grab'ить keysym'ы.
                // Без этого на свежезагруженной системе кириллические keybindings
                // (`<Control>Cyrillic_io` для Ё) НЕ срабатывают пока пользователь
                // не переключит раскладку хотя бы раз вручную (Super+Space).
                // setxkbmap с теми же параметрами триггерит XKB-reload и mutter
                // пересоздаёт grabs, включая Cyrillic_io.
                let is_x11 = std::env::var("XDG_SESSION_TYPE")
                    .map(|v| v == "x11")
                    .unwrap_or(false);
                if is_x11
                    && let Ok(query) = std::process::Command::new("setxkbmap").arg("-query").output()
                {
                    let mut layout = String::new();
                    let mut variant = String::new();
                    for line in String::from_utf8_lossy(&query.stdout).lines() {
                        if let Some(rest) = line.strip_prefix("layout:") {
                            layout = rest.trim().to_string();
                        } else if let Some(rest) = line.strip_prefix("variant:") {
                            variant = rest.trim().to_string();
                        }
                    }
                    if !layout.is_empty() {
                        let mut args = vec!["-layout".to_string(), layout.clone()];
                        if !variant.is_empty() {
                            args.push("-variant".to_string());
                            args.push(variant.clone());
                        }
                        let _ = std::process::Command::new("setxkbmap").args(&args).output();
                        tracing::info!(
                            "XKB пнут (layout={}, variant={}) — mutter пере-grab'ит cyr keysym'ы",
                            layout, variant
                        );
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
        .invoke_handler(tauri::generate_handler![trigger, pause, cancel_transcription, active_supports_cancel, get_audio_level, is_recording, is_paused, is_model_loaded, get_loaded_models, get_compiled_engines, get_cpu_features, get_default_input_device_name, get_models, download_model, delete_model, is_wayland, check_hotkey_conflict, register_gnome_hotkeys, hide_window, load_config, save_config, set_history_filter, set_language, get_history, delete_history_entry, clear_history, export_history, retranscribe, get_audio_data])
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
