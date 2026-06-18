// crates/arcanaglyph-app/src/models/registry.rs
//
// Tauri-команды для управления реестром моделей: get_models (с проверкой
// наличия и подбором display-пути), delete_model (удаляет файлы + чистит
// совпадающий путь в config), download_model (старт ручной загрузки из UI).
// Проверка «модель установлена» живёт в нейтральном `installed` (разрыв цикла
// `download ↔ registry`).

use super::download::{download_file, extract_zip_into_model_dir};
use super::installed::is_model_installed;
use arcanaglyph_core::CoreConfig;

/// Резолвит путь к файлу модели для проверки наличия и отображения в UI.
///
/// Для Whisper в реестре две модели (Tiny + Large), но `whisper_model_path` в config
/// один. Если бы для обеих карточек брали config_path, одна показывала бы статус
/// другой (Large card видела бы `ggml-tiny.bin` → размер не совпадает с expected →
/// ложное «Не найдена», хотя файл Large лежит в `models_base_dir/...`). Решение:
/// **для Whisper — всегда `models_base_dir/<default_filename>` per-model**, независимо
/// от config_path (сам config_path engine использует для загрузки активной модели).
///
/// Для Vosk/GigaAM/Qwen3-ASR — одна модель на движок: берём config-путь, а если он
/// пуст — fallback на дефолтную локацию. Чистая функция — тестируема без диска.
fn resolve_model_file_path(transcriber_type: &str, default_filename: &str, config: &CoreConfig) -> std::path::PathBuf {
    if transcriber_type == "whisper" {
        return config.models_base_dir.join(default_filename);
    }
    let config_path = match transcriber_type {
        "gigaam" => &config.gigaam_model_path,
        "gigaam-rnnt" => &config.gigaam_rnnt_model_path,
        "qwen3asr" => &config.qwen3asr_model_path,
        // vosk и неизвестные движки исторически читают `model_path`.
        _ => &config.model_path,
    };
    if config_path.as_os_str().is_empty() {
        config.models_base_dir.join(default_filename)
    } else {
        config_path.clone()
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
            // Резолвим путь к ФАЙЛУ модели (для проверки наличия + UI display) —
            // подробности per-движок в doc `resolve_model_file_path`.
            let resolved_path = resolve_model_file_path(m.transcriber_type, m.default_filename, &config);
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

/// Удаляет файлы модели с диска (idempotent — отсутствие файла не ошибка). Whisper
/// хранит сам `.bin`-файл → `remove_file`; директорные движки (Vosk/GigaAM/Qwen3-ASR)
/// → `remove_dir_all`. Вынесено из `delete_model` ради снижения сложности. Async (fs)
/// — не юнит-тестируется (проверяется live).
async fn remove_model_from_disk(transcriber_type: &str, pb: &std::path::Path) -> Result<(), String> {
    if !pb.exists() {
        return Ok(());
    }
    // Директорные движки удаляют дерево; whisper (и вырожденный «не директория») — файл.
    if transcriber_type != "whisper" && pb.is_dir() {
        tokio::fs::remove_dir_all(pb)
            .await
            .map_err(|e| format!("Не удалось удалить директорию {}: {}", pb.display(), e))?;
        tracing::info!("Удалена директория модели: {}", pb.display());
    } else {
        tokio::fs::remove_file(pb)
            .await
            .map_err(|e| format!("Не удалось удалить файл {}: {}", pb.display(), e))?;
        tracing::info!("Удалён файл модели: {}", pb.display());
    }
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
pub async fn delete_model(model_id: String, path: String) -> Result<(), String> {
    let model_info = arcanaglyph_core::transcription_models::find(&model_id)
        .ok_or_else(|| format!("Модель '{}' не найдена в реестре", model_id))?;
    let pb = std::path::PathBuf::from(&path);

    remove_model_from_disk(model_info.transcriber_type, &pb).await?;

    // Чистим config-путь, если он совпадает с удалённым.
    let mut config = CoreConfig::load().map_err(|e| e.to_string())?;
    if clear_model_config_path_if_matches(&mut config, model_info.transcriber_type, &pb) {
        config.save().map_err(|e| e.to_string())?;
        tracing::info!(
            "Очищен config-путь для движка '{}' после удаления модели",
            model_info.transcriber_type
        );
    }
    Ok(())
}

/// Путь, который считается «установленной моделью» для движка: Whisper хранит сам
/// `.bin`-файл, остальные движки (Vosk/GigaAM/Qwen3-ASR) — директорию модели.
/// Чистая функция — единый выбор для guard'а установки и обновления config-пути
/// (раньше дублировался в `download_model` дважды). Тестируема без tauri.
fn model_install_path(
    transcriber_type: &str,
    main_dest: &std::path::Path,
    dest_path: &std::path::Path,
) -> std::path::PathBuf {
    if transcriber_type == "whisper" {
        main_dest.to_path_buf()
    } else {
        dest_path.to_path_buf()
    }
}

/// Проставляет в `config` путь скачанной модели в поле, соответствующее движку.
/// Возвращает `true`, если движок известен и поле обновлено (вызывающий тогда
/// сохраняет config); `false` для неизвестного типа. Чистая функция — мутирует
/// config без I/O, тестируема без tauri.
fn apply_model_config_path(config: &mut CoreConfig, transcriber_type: &str, path: std::path::PathBuf) -> bool {
    match transcriber_type {
        "vosk" => config.model_path = path,
        "whisper" => config.whisper_model_path = path,
        "gigaam" => config.gigaam_model_path = path,
        "gigaam-rnnt" => config.gigaam_rnnt_model_path = path,
        "qwen3asr" => config.qwen3asr_model_path = path,
        _ => return false,
    }
    true
}

/// Затирает config-путь движка на пустой, ЕСЛИ он совпадает с удалённым `deleted_path`.
/// Возвращает `true`, если поле было очищено (вызывающий тогда сохраняет config).
/// Несколько моделей одного движка (Whisper Tiny+Large): чистим только при точном
/// совпадении пути — иначе удаление одной обнулило бы путь к оставшейся. Чистая
/// функция — мутирует config без I/O, тестируема.
fn clear_model_config_path_if_matches(
    config: &mut CoreConfig,
    transcriber_type: &str,
    deleted_path: &std::path::Path,
) -> bool {
    let field = match transcriber_type {
        "vosk" => &mut config.model_path,
        "whisper" => &mut config.whisper_model_path,
        "gigaam" => &mut config.gigaam_model_path,
        "gigaam-rnnt" => &mut config.gigaam_rnnt_model_path,
        "qwen3asr" => &mut config.qwen3asr_model_path,
        _ => return false,
    };
    if field == deleted_path {
        *field = std::path::PathBuf::new();
        true
    } else {
        false
    }
}

/// Обновляет config-путь движка на скачанный файл и сохраняет config. Вынесено из
/// `download_model` ради снижения вложенности orchestrator'а. Зачем нужно: после
/// `delete_model` config-путь пуст, и без этого скачанная заново модель считалась бы
/// «не выбранной»; также позволяет переключать варианты Whisper (Tiny/Large) одним
/// «Скачать». Ошибка сохранения не фатальна (файлы уже на диске) — только логируется.
/// Ядро выбора поля — чистый `apply_model_config_path` (покрыт тестами).
fn persist_downloaded_model_path(transcriber_type: &str, main_dest: &std::path::Path, dest_path: &std::path::Path) {
    let saved_path = model_install_path(transcriber_type, main_dest, dest_path);
    let Ok(mut config) = CoreConfig::load() else {
        return;
    };
    if !apply_model_config_path(&mut config, transcriber_type, saved_path) {
        return;
    }
    match config.save() {
        Err(e) => tracing::warn!("Не удалось сохранить config-путь после скачивания: {}", e),
        Ok(()) => tracing::info!(
            "Config-путь для движка '{}' обновлён на скачанный файл",
            transcriber_type
        ),
    }
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
    let _ = tokio::fs::create_dir_all(&dest_path).await;

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
            let install_path = model_install_path(m.transcriber_type, &main_dest, &dest_path);
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
        let zip_meta = tokio::fs::metadata(&main_dest).await;
        let zip_already_downloaded = main_dest.extension().and_then(|s| s.to_str()) == Some("zip")
            && match (min_size, zip_meta) {
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

    // Авто-обновление config-пути для скачанной модели (детали — в doc хелпера).
    if let Some(model_info) = model_info {
        persist_downloaded_model_path(model_info.transcriber_type, &main_dest, &dest_path);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_model_install_path_whisper_is_file_others_dir() {
        let main_dest = PathBuf::from("/models/ggml-large-v3-turbo.bin");
        let dest_dir = PathBuf::from("/models/whisper-dir");
        // Whisper → сам .bin-файл.
        assert_eq!(model_install_path("whisper", &main_dest, &dest_dir), main_dest);
        // Остальные движки → директория модели.
        for t in ["vosk", "gigaam", "gigaam-rnnt", "qwen3asr", "totally-unknown"] {
            assert_eq!(model_install_path(t, &main_dest, &dest_dir), dest_dir);
        }
    }

    #[test]
    fn test_apply_model_config_path_sets_right_field() {
        let p = PathBuf::from("/models/downloaded");

        let mut config = CoreConfig::default();
        assert!(apply_model_config_path(&mut config, "vosk", p.clone()));
        assert_eq!(config.model_path, p);

        let mut config = CoreConfig::default();
        assert!(apply_model_config_path(&mut config, "whisper", p.clone()));
        assert_eq!(config.whisper_model_path, p);

        let mut config = CoreConfig::default();
        assert!(apply_model_config_path(&mut config, "gigaam", p.clone()));
        assert_eq!(config.gigaam_model_path, p);

        let mut config = CoreConfig::default();
        assert!(apply_model_config_path(&mut config, "gigaam-rnnt", p.clone()));
        assert_eq!(config.gigaam_rnnt_model_path, p);

        let mut config = CoreConfig::default();
        assert!(apply_model_config_path(&mut config, "qwen3asr", p.clone()));
        assert_eq!(config.qwen3asr_model_path, p);
    }

    #[test]
    fn test_apply_model_config_path_unknown_returns_false_and_no_mutation() {
        let mut config = CoreConfig::default();
        let before = config.model_path.clone();
        // Неизвестный движок → false, ни одно поле не тронуто.
        assert!(!apply_model_config_path(
            &mut config,
            "totally-unknown",
            PathBuf::from("/x")
        ));
        assert_eq!(config.model_path, before);
    }

    #[test]
    fn test_clear_model_config_path_if_matches() {
        let p = PathBuf::from("/models/gigaam-v3");
        // Совпадает → поле очищено, true.
        let mut config = CoreConfig {
            gigaam_model_path: p.clone(),
            ..Default::default()
        };
        assert!(clear_model_config_path_if_matches(&mut config, "gigaam", &p));
        assert_eq!(config.gigaam_model_path, PathBuf::new());

        // Не совпадает (другая модель того же движка) → не трогаем, false.
        let other = PathBuf::from("/models/gigaam-other");
        let mut config = CoreConfig {
            gigaam_model_path: other.clone(),
            ..Default::default()
        };
        assert!(!clear_model_config_path_if_matches(&mut config, "gigaam", &p));
        assert_eq!(config.gigaam_model_path, other);

        // Неизвестный движок → false.
        let mut config = CoreConfig::default();
        assert!(!clear_model_config_path_if_matches(&mut config, "totally-unknown", &p));
    }

    #[test]
    fn test_resolve_model_file_path_whisper_always_base_dir() {
        // Whisper всегда base_dir/<filename>, даже если whisper_model_path задан.
        let config = CoreConfig {
            models_base_dir: PathBuf::from("/base"),
            whisper_model_path: PathBuf::from("/somewhere/ggml-tiny.bin"),
            ..Default::default()
        };
        assert_eq!(
            resolve_model_file_path("whisper", "ggml-large-v3-turbo.bin", &config),
            PathBuf::from("/base/ggml-large-v3-turbo.bin")
        );
    }

    #[test]
    fn test_resolve_model_file_path_uses_config_or_fallback() {
        let config = CoreConfig {
            models_base_dir: PathBuf::from("/base"),
            gigaam_model_path: PathBuf::from("/custom/gigaam"),
            gigaam_rnnt_model_path: PathBuf::new(), // явно пусто → fallback
            ..Default::default()
        };
        // Непустой config-путь движка → берётся как есть.
        assert_eq!(
            resolve_model_file_path("gigaam", "gigaam-v3-e2e-ctc", &config),
            PathBuf::from("/custom/gigaam")
        );
        // Пустой config-путь → fallback на base_dir/<filename>.
        assert_eq!(
            resolve_model_file_path("gigaam-rnnt", "gigaam-v3-e2e-rnnt", &config),
            PathBuf::from("/base/gigaam-v3-e2e-rnnt")
        );
    }
}
