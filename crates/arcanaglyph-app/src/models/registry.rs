// crates/arcanaglyph-app/src/models/registry.rs
//
// Tauri-команды для управления реестром моделей: get_models (с проверкой
// наличия и подбором display-пути), delete_model (удаляет файлы + чистит
// совпадающий путь в config), download_model (старт ручной загрузки из UI).
// Хелпер `is_model_installed` живёт здесь и переиспользуется `download::ensure_active_model`.

use super::download::{download_file, extract_zip_into_model_dir};
use arcanaglyph_core::CoreConfig;

/// Проверяет, установлена ли модель (не просто существование директории, а ключевые файлы).
///
/// `min_size` — минимальный ожидаемый размер главного файла. Если задан и фактический
/// размер меньше — считаем, что файл повреждён (например, прерванное скачивание).
pub(crate) fn is_model_installed(path: &std::path::Path, transcriber_type: &str, min_size: Option<u64>) -> bool {
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
pub fn get_models() -> Result<serde_json::Value, String> {
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
pub async fn delete_model(model_id: String, path: String) -> Result<(), String> {
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
pub async fn download_model(
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
