// crates/arcanaglyph-app/src/models/download.rs
//
// Скачивание моделей с прогресс-эвентами, распаковка zip (Vosk) и
// `ensure_active_model` — гарантирует наличие модели активного движка на
// диске перед стартом engine. Все ошибки преобразуются в `String`, потому
// что вызывающий код (Tauri-spawn и команда `download_model`) ожидает
// именно `Result<_, String>`.

use super::installed::is_model_installed;
use arcanaglyph_core::CoreConfig;

/// Контекст пакетной загрузки одной модели: инварианты, общие для главного файла
/// и всех extra-файлов (одинаковы во всех вызовах `download_file` в пределах модели).
/// Собирается один раз перед скачиванием и прокидывается в каждый вызов —
/// снимает повторную передачу `model_id`/`total_files`/`app` на каждом файле.
pub(crate) struct DownloadCtx<'a> {
    /// Хендл приложения для эмита `download://progress`.
    pub app: &'a tauri::AppHandle,
    /// Идентификатор модели (в payload прогресса).
    pub model_id: &'a str,
    /// Всего файлов в загрузке модели (для индикатора `[idx/total]`).
    pub total_files: usize,
}

/// Скачивание одного файла с прогрессом.
///
/// Атомарность: пишем в `<dest>.partial` и переименовываем по успеху. Если процесс
/// прервётся (Ctrl+C, обрыв сети, нехватка места) — целевой файл не появится,
/// и следующий запуск перекачает с нуля. Это лечит «фантомно установленные» модели,
/// которые потом валятся в whisper_model_load с «expected N tensors, got 5».
///
/// `min_size` — необязательный минимальный размер в байтах. Если задан и фактический
/// размер меньше — возвращаем ошибку, файл `.partial` остаётся (перекачается заново).
/// Стримит тело `response` в открытый `file`, эмитя `download://progress` на каждом
/// изменении процента. Возвращает число фактически скачанных байт (для последующей
/// валидации размера). Вынесено из `download_file` — держит его цикл-скачивание отдельно
/// от setup/flush/rename.
async fn stream_to_file(
    ctx: &DownloadCtx<'_>,
    response: reqwest::Response,
    file: &mut tokio::fs::File,
    filename: &str,
    file_idx: usize,
) -> Result<u64, String> {
    use futures_util::StreamExt;
    use tauri::Emitter;

    let total_size = response.content_length().unwrap_or(0);
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_progress: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Ошибка скачивания {}: {}", filename, e))?;
        tokio::io::AsyncWriteExt::write_all(file, &chunk)
            .await
            .map_err(|e| format!("Ошибка записи {}: {}", filename, e))?;

        downloaded += chunk.len() as u64;
        let progress_pct = (downloaded * 100).checked_div(total_size).unwrap_or(0);
        if progress_pct != last_progress {
            last_progress = progress_pct;
            let _ = ctx.app.emit(
                "download://progress",
                serde_json::json!({
                    "model_id": ctx.model_id,
                    "file": filename,
                    "file_idx": file_idx,
                    "total_files": ctx.total_files,
                    "downloaded": downloaded,
                    "total": total_size,
                    "percent": progress_pct,
                }),
            );
        }
    }
    Ok(downloaded)
}

pub(crate) async fn download_file(
    ctx: &DownloadCtx<'_>,
    url: &str,
    dest: &std::path::Path,
    file_idx: usize,
    min_size: Option<u64>,
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
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
        ctx.total_files,
        filename,
        dest.display()
    );

    let response = reqwest::get(url)
        .await
        .map_err(|e| format!("Ошибка запроса {}: {}", filename, e))?;

    let mut file = tokio::fs::File::create(&partial)
        .await
        .map_err(|e| format!("Не удалось создать {}: {}", partial.display(), e))?;

    let downloaded = stream_to_file(ctx, response, &mut file, filename, file_idx).await?;

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
pub(crate) async fn extract_zip_into_model_dir(
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

    if let Err(e) = tokio::fs::remove_file(zip_path).await {
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

/// Резолвит путь к модели активного движка из конфига. `None` — неизвестный тип
/// движка: вызывающий код тогда просто выходит (engine create вернёт нормальную ошибку).
/// Чистая функция — тестируема без диска.
fn resolve_active_model_path(cfg: &CoreConfig, transcriber_type: &str) -> Option<std::path::PathBuf> {
    let path = match transcriber_type {
        "vosk" => &cfg.model_path,
        "whisper" => &cfg.whisper_model_path,
        "gigaam" => &cfg.gigaam_model_path,
        "gigaam-rnnt" => &cfg.gigaam_rnnt_model_path,
        "qwen3asr" => &cfg.qwen3asr_model_path,
        _ => return None,
    };
    Some(path.clone())
}

/// Выбирает запись реестра для движка. Для Whisper в реестре несколько моделей
/// (Tiny/Small/Large) с разными размерами — выбираем ту, чей `default_filename`
/// совпадает с именем файла в config-пути, иначе валидация может ошибочно посчитать
/// Tiny-файл (~75 МБ) повреждённым по порогу Small (~400 МБ) или наоборот. Для
/// остальных движков — одна модель на тип. Чистая функция — тестируема без диска.
fn select_registry_model(
    transcriber_type: &str,
    filename: &str,
) -> Option<&'static arcanaglyph_core::transcription_models::SpeechModelInfo> {
    use arcanaglyph_core::transcription_models;
    if transcriber_type == "whisper" && !filename.is_empty() {
        transcription_models::find_by_type_and_filename(transcriber_type, filename)
    } else {
        transcription_models::find_by_transcriber_type(transcriber_type)
    }
}

/// Вычисляет путь к главному файлу модели и (для не-Whisper движков) создаёт
/// директорию модели. Whisper: путь — это сам файл (`whisper_model_path` указывает
/// прямо на .bin). Остальные движки: путь — это директория; имя главного файла берём
/// из последнего сегмента URL (а не из `default_filename` — это имя ДИРЕКТОРИИ, не
/// файла; смешение сломало бы сохранение скачанной модели).
async fn prepare_main_dest(
    transcriber_type: &str,
    model_dir: &std::path::Path,
    model: &arcanaglyph_core::transcription_models::SpeechModelInfo,
) -> std::path::PathBuf {
    if transcriber_type == "whisper" {
        model_dir.to_path_buf()
    } else {
        let _ = tokio::fs::create_dir_all(model_dir).await;
        let main_filename = model.download_url.rsplit('/').next().unwrap_or("model.bin");
        model_dir.join(main_filename)
    }
}

/// Чистит повреждённый главный файл (меньше порога `min_size`), чтобы не путать
/// atomic-rename при перекачке.
async fn remove_corrupted_main_file(main_dest: &std::path::Path, min_size: Option<u64>) {
    let main_meta = tokio::fs::metadata(main_dest).await;
    if let (Some(min), Ok(meta)) = (min_size, main_meta)
        && meta.is_file()
        && meta.len() < min
    {
        tracing::warn!(
            "Файл модели повреждён (размер {} байт, минимум {}) — будет перекачан: {}",
            meta.len(),
            min,
            main_dest.display()
        );
        let _ = tokio::fs::remove_file(main_dest).await;
    }
}

/// Что и куда скачивать в рамках одной модели — группирует параметры `download_model_files`
/// (Introduce Parameter Object). `model_id` намеренно не дублируется: он берётся из `DownloadCtx`.
pub(crate) struct ModelDownloadSpec<'a> {
    /// URL главного файла модели.
    pub url: &'a str,
    /// Куда сохранить главный файл.
    pub main_dest: &'a std::path::Path,
    /// Директория модели (цель распаковки zip и база для extra-файлов).
    pub model_dir: &'a std::path::Path,
    /// Дополнительные файлы: `(url, относительный путь внутри model_dir)`.
    pub extras: &'a [(&'a str, &'a str)],
    /// Минимальный размер главного файла (валидация + skip уже скачанного zip).
    pub min_size: Option<u64>,
}

/// Скачивает главный файл модели (со skip'ом уже валидного zip), при необходимости
/// распаковывает zip и докачивает extra-файлы. Прогресс/распаковка эмитятся внутри
/// `download_file` / `extract_zip_into_model_dir`. Единый источник для `ensure_active_model`
/// (типовой путь: url/id/extras берутся из реестра) и `registry::download_model` (ручная
/// загрузка из UI, где url приходит параметром и модели может не быть в реестре).
pub(crate) async fn download_model_files(ctx: &DownloadCtx<'_>, spec: &ModelDownloadSpec<'_>) -> Result<(), String> {
    // Тот же skip как в download_model: если zip уже скачан и валиден по размеру,
    // не перекачиваем 1.8 ГБ ради повторной попытки распаковки.
    let zip_meta = tokio::fs::metadata(spec.main_dest).await;
    let zip_already_downloaded = spec.main_dest.extension().and_then(|s| s.to_str()) == Some("zip")
        && match (spec.min_size, zip_meta) {
            (Some(min), Ok(m)) if m.is_file() => m.len() >= min,
            _ => false,
        };
    if zip_already_downloaded {
        tracing::info!(
            "zip-архив {} уже на диске и валидного размера — пропускаем загрузку",
            spec.main_dest.display()
        );
    } else {
        download_file(ctx, spec.url, spec.main_dest, 0, spec.min_size).await?;
    }
    // Если основной файл — .zip-архив, распаковываем сразу после скачивания.
    // Сейчас касается только Vosk; для будущих архивных моделей сработает автоматом.
    if spec.main_dest.extension().and_then(|s| s.to_str()) == Some("zip") {
        extract_zip_into_model_dir(spec.main_dest, spec.model_dir, ctx.model_id, ctx.app).await?;
    }
    for (idx, (extra_url, rel)) in spec.extras.iter().enumerate() {
        let extra_dest = spec.model_dir.join(rel);
        // Для extra-файлов size-проверка не задаётся: размеры варьируются и порог в реестре пока один общий.
        download_file(ctx, extra_url, &extra_dest, idx + 1, None).await?;
    }
    Ok(())
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
pub async fn ensure_active_model(transcriber_type: &str, app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Emitter;

    let cfg = CoreConfig::load().map_err(|e| e.to_string())?;
    let Some(path) = resolve_active_model_path(&cfg, transcriber_type) else {
        return Ok(()); // unknown тип — пусть engine create вернёт нормальную ошибку
    };

    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let model = select_registry_model(transcriber_type, filename);
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

    let main_dest = prepare_main_dest(transcriber_type, &path, model).await;
    remove_corrupted_main_file(&main_dest, min_size).await;

    let ctx = DownloadCtx {
        app,
        model_id: model.id,
        total_files: 1 + model.extra_files.unwrap_or(&[]).len(),
    };
    let spec = ModelDownloadSpec {
        url: model.download_url,
        main_dest: &main_dest,
        model_dir: &path,
        extras: model.extra_files.unwrap_or(&[]),
        min_size,
    };
    download_model_files(&ctx, &spec).await?;

    let _ = app.emit("download://complete", serde_json::json!({ "model_id": model.id }));
    tracing::info!("Модель '{}' успешно установлена", model.display_name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcanaglyph_core::transcription_models;
    use std::path::PathBuf;

    #[test]
    fn resolve_active_model_path_maps_each_engine() {
        let cfg = CoreConfig {
            model_path: PathBuf::from("/m/vosk"),
            whisper_model_path: PathBuf::from("/m/whisper.bin"),
            gigaam_model_path: PathBuf::from("/m/gigaam"),
            gigaam_rnnt_model_path: PathBuf::from("/m/rnnt"),
            qwen3asr_model_path: PathBuf::from("/m/qwen"),
            ..Default::default()
        };
        assert_eq!(resolve_active_model_path(&cfg, "vosk"), Some(PathBuf::from("/m/vosk")));
        assert_eq!(
            resolve_active_model_path(&cfg, "whisper"),
            Some(PathBuf::from("/m/whisper.bin"))
        );
        assert_eq!(
            resolve_active_model_path(&cfg, "gigaam"),
            Some(PathBuf::from("/m/gigaam"))
        );
        assert_eq!(
            resolve_active_model_path(&cfg, "gigaam-rnnt"),
            Some(PathBuf::from("/m/rnnt"))
        );
        assert_eq!(
            resolve_active_model_path(&cfg, "qwen3asr"),
            Some(PathBuf::from("/m/qwen"))
        );
    }

    #[test]
    fn resolve_active_model_path_unknown_is_none() {
        assert_eq!(
            resolve_active_model_path(&CoreConfig::default(), "totally-unknown"),
            None
        );
    }

    #[test]
    fn select_registry_model_non_whisper_by_type() {
        // Не-Whisper: одна модель на тип, filename игнорируется.
        let m = select_registry_model("gigaam", "");
        assert!(m.is_some(), "gigaam модель должна быть в реестре");
        assert_eq!(m.unwrap().transcriber_type, "gigaam");
    }

    // Whisper-модели за cargo-feature — в тестовой сборке их может не быть в реестре,
    // поэтому проверяем не наличие модели, а КОРРЕКТНОСТЬ ДЕЛЕГАЦИИ (какую ветку выбирает
    // select_registry_model). Инвариант держится и когда обе стороны дают None.
    #[test]
    fn select_registry_model_whisper_with_filename_delegates_by_file() {
        let expected = transcription_models::find_by_type_and_filename("whisper", "ggml-large-v3-turbo.bin");
        let got = select_registry_model("whisper", "ggml-large-v3-turbo.bin");
        assert_eq!(got.map(|m| m.id), expected.map(|m| m.id));
    }

    #[test]
    fn select_registry_model_whisper_empty_filename_delegates_by_type() {
        let expected = transcription_models::find_by_transcriber_type("whisper");
        let got = select_registry_model("whisper", "");
        assert_eq!(got.map(|m| m.id), expected.map(|m| m.id));
    }

    #[test]
    fn select_registry_model_unknown_is_none() {
        assert!(select_registry_model("totally-unknown", "").is_none());
    }
}
