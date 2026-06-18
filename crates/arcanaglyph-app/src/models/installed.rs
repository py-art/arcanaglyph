// crates/arcanaglyph-app/src/models/installed.rs
//
// Проверка «модель установлена на диске» — ключевые файлы движка, а не просто
// существование директории. Вынесено в отдельный нейтральный модуль, чтобы и
// `registry` (guard в download_model, get_models), и `download::ensure_active_model`
// зависели от него, не образуя цикла `download ↔ registry`.

/// Главный файл модели, размер которого валидируется по `min_size`. Для whisper это
/// сам путь (он и есть файл), для директорных движков — ключевой файл внутри.
/// `None` — у движка нет одного «главного» файла для size-проверки (vosk/неизвестный).
/// Чистая функция (только склейка путей) — тестируема.
fn model_main_file(transcriber_type: &str, path: &std::path::Path) -> Option<std::path::PathBuf> {
    match transcriber_type {
        "whisper" => Some(path.to_path_buf()),
        "gigaam" => Some(path.join("v3_e2e_ctc.int8.onnx")),
        "gigaam-rnnt" => Some(path.join("v3_e2e_rnnt_encoder.int8.onnx")),
        "qwen3asr" => Some(path.join("tokenizer.json")),
        _ => None,
    }
}

/// Главный файл присутствует и (если задан `min_size`) не меньше ожидаемого размера.
/// `None`-файл считается валидным (у движка нет главного файла для size-проверки).
/// Меньший размер трактуем как повреждённый/недокачанный файл.
fn model_main_file_valid(main_file: Option<&std::path::Path>, min_size: Option<u64>) -> bool {
    let Some(f) = main_file else {
        return true;
    };
    if !f.exists() {
        return false;
    }
    let Some(min) = min_size else {
        return true;
    };
    matches!(std::fs::metadata(f), Ok(meta) if meta.is_file() && meta.len() >= min)
}

/// Дополнительные обязательные файлы движка присутствуют (без проверки размера —
/// только наличие). Для неизвестного типа доп-файлов нет → `true`. Чистая функция.
fn model_extra_files_present(transcriber_type: &str, path: &std::path::Path) -> bool {
    match transcriber_type {
        "gigaam" => path.join("v3_e2e_ctc_vocab.txt").exists(),
        "gigaam-rnnt" => {
            path.join("v3_e2e_rnnt_decoder.int8.onnx").exists()
                && path.join("v3_e2e_rnnt_joint.int8.onnx").exists()
                && path.join("v3_e2e_rnnt_vocab.txt").exists()
        }
        "qwen3asr" => path.join("onnx_models/encoder_conv.onnx").exists(),
        "vosk" => path.join("conf").exists() || path.join("am").exists() || path.join("graph").exists(),
        _ => true,
    }
}

/// Проверяет, установлена ли модель (не просто существование директории, а ключевые файлы).
///
/// `min_size` — минимальный ожидаемый размер главного файла. Если задан и фактический
/// размер меньше — считаем, что файл повреждён (например, прерванное скачивание).
pub(crate) fn is_model_installed(path: &std::path::Path, transcriber_type: &str, min_size: Option<u64>) -> bool {
    if !path.exists() {
        return false;
    }
    let main_file = model_main_file(transcriber_type, path);
    if !model_main_file_valid(main_file.as_deref(), min_size) {
        return false;
    }
    model_extra_files_present(transcriber_type, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Уникальная временная директория под тест (без внешних crate'ов, как `temp_db`).
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("arcanaglyph_installed_test_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_file(path: &std::path::Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, vec![0u8; bytes]).expect("write file");
    }

    #[test]
    fn test_missing_path_is_not_installed() {
        let missing = std::env::temp_dir().join("arcanaglyph_installed_does_not_exist_xyz");
        let _ = fs::remove_dir_all(&missing);
        assert!(!is_model_installed(&missing, "gigaam", None));
    }

    #[test]
    fn test_gigaam_requires_main_file_and_vocab() {
        let dir = temp_dir("gigaam");
        // Только onnx, без vocab → не установлена.
        write_file(&dir.join("v3_e2e_ctc.int8.onnx"), 1024);
        assert!(!is_model_installed(&dir, "gigaam", None));
        // Добавили vocab → установлена.
        write_file(&dir.join("v3_e2e_ctc_vocab.txt"), 16);
        assert!(is_model_installed(&dir, "gigaam", None));
        // Главный файл отсутствует → не установлена.
        fs::remove_file(dir.join("v3_e2e_ctc.int8.onnx")).unwrap();
        assert!(!is_model_installed(&dir, "gigaam", None));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_whisper_validates_min_size() {
        let dir = temp_dir("whisper");
        let model = dir.join("ggml-large-v3-turbo.bin");
        write_file(&model, 100);
        // Размер 100 >= min 50 → установлена.
        assert!(is_model_installed(&model, "whisper", Some(50)));
        // Размер 100 < min 1000 → повреждена/недокачана.
        assert!(!is_model_installed(&model, "whisper", Some(1000)));
        // Без min_size — достаточно существования файла.
        assert!(is_model_installed(&model, "whisper", None));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_unknown_type_only_checks_path_exists() {
        let dir = temp_dir("unknown");
        // Неизвестный тип: main_file = None, доп-файлов нет → достаточно наличия пути.
        assert!(is_model_installed(&dir, "totally-unknown", None));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_model_main_file_picks_engine_specific_path() {
        let base = PathBuf::from("/models/x");
        // Whisper: главный файл — сам путь.
        assert_eq!(model_main_file("whisper", &base), Some(base.clone()));
        // Директорные движки: ключевой файл внутри директории.
        assert_eq!(
            model_main_file("gigaam", &base),
            Some(base.join("v3_e2e_ctc.int8.onnx"))
        );
        assert_eq!(
            model_main_file("gigaam-rnnt", &base),
            Some(base.join("v3_e2e_rnnt_encoder.int8.onnx"))
        );
        assert_eq!(model_main_file("qwen3asr", &base), Some(base.join("tokenizer.json")));
        // Vosk / неизвестный — нет одного главного файла для size-проверки.
        assert_eq!(model_main_file("vosk", &base), None);
        assert_eq!(model_main_file("totally-unknown", &base), None);
    }

    #[test]
    fn test_model_main_file_valid_none_and_size_checks() {
        // None-файл (vosk) → валиден без size-проверки.
        assert!(model_main_file_valid(None, Some(1000)));

        let dir = temp_dir("mainvalid");
        let f = dir.join("model.bin");
        write_file(&f, 100);
        // Существует, без min_size → валиден.
        assert!(model_main_file_valid(Some(&f), None));
        // 100 >= 50 → валиден; 100 < 1000 → невалиден (повреждён/недокачан).
        assert!(model_main_file_valid(Some(&f), Some(50)));
        assert!(!model_main_file_valid(Some(&f), Some(1000)));
        // Несуществующий файл → невалиден.
        assert!(!model_main_file_valid(Some(&dir.join("nope.bin")), None));
        let _ = fs::remove_dir_all(&dir);
    }
}
