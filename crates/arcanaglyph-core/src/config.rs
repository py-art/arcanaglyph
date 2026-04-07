// crates/arcanaglyph-core/src/config.rs

use crate::error::ArcanaError;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Движок транскрибации
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TranscriberType {
    /// Vosk — быстрый, потоковый, менее точный
    #[default]
    Vosk,
    /// Whisper — медленнее, значительно точнее
    Whisper,
    /// GigaAM v3 — лучший для русского (ONNX, SberDevices)
    GigaAm,
}

/// Конфигурация ядра ArcanaGlyph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    /// Движок транскрибации: vosk, whisper
    #[serde(default)]
    pub transcriber: TranscriberType,
    /// Путь к Vosk-модели (для transcriber = "vosk")
    pub model_path: PathBuf,
    /// Путь к Whisper-модели в формате ggml (для transcriber = "whisper")
    /// Доступные модели (скачать с HuggingFace ggerganov/whisper.cpp):
    ///   ggml-large-v3-turbo.bin  — лучший баланс скорости/качества (~1.5 ГБ)
    ///   ggml-large-v3.bin        — максимальное качество, медленнее (~3 ГБ)
    ///   ggml-medium.bin          — средний вариант (~1.5 ГБ)
    ///   ggml-small.bin           — быстрый, менее точный (~500 МБ)
    #[serde(default = "default_whisper_model_path")]
    pub whisper_model_path: PathBuf,
    /// Частота дискретизации аудио (Гц)
    pub sample_rate: u32,
    /// Таймаут тишины (секунды): если нет новых слов столько времени — запись автоматически останавливается
    pub max_record_secs: u64,
    /// Автоматически вставлять распознанный текст в активное окно
    pub auto_type: bool,
    /// Горячая клавиша для триггера (формат Tauri: "Super+Alt+Control+Space")
    pub hotkey: String,
    /// Горячая клавиша для паузы (формат Tauri, пустая строка = не задана)
    #[serde(default)]
    pub hotkey_pause: String,
    /// Режим отладки: выводить промежуточные результаты распознавания в терминал
    pub debug: bool,
    /// Путь к директории GigaAM-модели (для transcriber = "gigaam")
    /// Директория должна содержать v3_e2e_ctc.int8.onnx и v3_e2e_ctc_vocab.txt
    #[serde(default = "default_gigaam_model_path")]
    pub gigaam_model_path: PathBuf,
    /// Авто-стоп записи при тишине после речи
    #[serde(default = "default_true")]
    pub vad_enabled: bool,
    /// Секунды тишины после речи для авто-стопа (если vad_enabled)
    #[serde(default = "default_vad_silence_secs")]
    pub vad_silence_secs: u64,
    /// Удалять слова-паразиты из транскрибации (э, э-э, ээ, эм, мм)
    #[serde(default = "default_true")]
    pub remove_fillers: bool,
    /// Срок хранения записей в часах (0 = хранить вечно)
    #[serde(default)]
    pub retention_hours: u64,
    /// Запускать в свёрнутом виде (сразу в трей)
    #[serde(default)]
    pub start_minimized: bool,
    /// Модели для предзагрузки при старте (помимо основной)
    #[serde(default)]
    pub preload_models: Vec<TranscriberType>,
}

fn default_whisper_model_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("models/ggml-large-v3-turbo.bin")
}

fn default_true() -> bool {
    true
}

fn default_vad_silence_secs() -> u64 {
    7
}

fn default_gigaam_model_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("models/gigaam-v3-e2e-ctc")
}

impl Default for CoreConfig {
    fn default() -> Self {
        // Пытаемся найти модель относительно текущей директории
        let model_path = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("models/vosk-model-ru-0.42");

        let whisper_model_path = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("models/ggml-large-v3-turbo.bin");

        let gigaam_model_path = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("models/gigaam-v3-e2e-ctc");

        Self {
            transcriber: TranscriberType::Vosk,
            model_path,
            whisper_model_path,
            gigaam_model_path,
            sample_rate: 48000,
            max_record_secs: 20,
            auto_type: true,
            hotkey: "Super+W".to_string(),
            hotkey_pause: "Super+Shift+W".to_string(),
            debug: true,
            vad_enabled: true,
            vad_silence_secs: 7,
            remove_fillers: true,
            retention_hours: 0,
            start_minimized: false,
            preload_models: vec![],
        }
    }
}

impl CoreConfig {
    /// Дефолтный конфиг с Whisper по умолчанию (для новых пользователей)
    pub fn default_whisper() -> Self {
        Self {
            transcriber: TranscriberType::Whisper,
            ..Self::default()
        }
    }

    /// Путь к конфигурационному файлу (legacy): ~/.config/arcanaglyph/config.toml
    pub fn config_path() -> Option<PathBuf> {
        ProjectDirs::from("com", "arcanaglyph", "ArcanaGlyph").map(|dirs| dirs.config_dir().join("config.toml"))
    }

    /// Путь к базе данных истории: ~/.config/arcanaglyph/history.db
    pub fn history_db_path() -> Option<PathBuf> {
        ProjectDirs::from("com", "arcanaglyph", "ArcanaGlyph").map(|dirs| dirs.config_dir().join("history.db"))
    }

    /// Директория кэша аудио: ~/.cache/arcanaglyph/audio/
    pub fn audio_cache_dir() -> Option<PathBuf> {
        ProjectDirs::from("com", "arcanaglyph", "ArcanaGlyph").map(|dirs| dirs.cache_dir().join("audio"))
    }

    /// Название текущей модели (для записи в историю)
    pub fn transcriber_model_name(&self) -> String {
        match self.transcriber {
            TranscriberType::Vosk => self.model_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "vosk".to_string()),
            TranscriberType::Whisper => self.whisper_model_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "whisper".to_string()),
            TranscriberType::GigaAm => self.gigaam_model_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "gigaam".to_string()),
        }
    }

    /// Тип транскрайбера как строка
    pub fn transcriber_type_str(&self) -> String {
        match self.transcriber {
            TranscriberType::Vosk => "vosk".to_string(),
            TranscriberType::Whisper => "whisper".to_string(),
            TranscriberType::GigaAm => "gigaam".to_string(),
        }
    }

    /// Загружает конфигурацию из SQLite БД. При первом запуске импортирует из config.toml если есть.
    pub fn load() -> Result<Self, ArcanaError> {
        let db_path = Self::history_db_path()
            .ok_or_else(|| ArcanaError::Config("Не удалось определить путь к БД".into()))?;
        let audio_cache = Self::audio_cache_dir()
            .ok_or_else(|| ArcanaError::Config("Не удалось определить путь к кэшу".into()))?;

        // Открываем БД (применяет миграции, создаёт таблицу settings)
        let db = crate::history::HistoryDB::new(&db_path, audio_cache)?;

        // Пробуем загрузить из SQLite
        if let Some(json_str) = db.get_setting("core_config") {
            let config: CoreConfig = serde_json::from_str(&json_str)
                .map_err(|e| ArcanaError::Config(format!("Ошибка парсинга конфига из БД: {}", e)))?;
            tracing::info!("Конфигурация загружена из БД");
            return Ok(config);
        }

        // Нет настроек в БД — пробуем импортировать из config.toml
        let config = if let Some(config_path) = Self::config_path() {
            if config_path.exists() {
                let content = std::fs::read_to_string(&config_path)
                    .map_err(|e| ArcanaError::Config(format!("Не удалось прочитать {}: {}", config_path.display(), e)))?;
                let config: CoreConfig = toml::from_str(&content)
                    .map_err(|e| ArcanaError::Config(format!("Ошибка парсинга config.toml: {}", e)))?;

                tracing::info!("Импорт настроек из config.toml");
                let _ = std::fs::remove_file(&config_path);
                tracing::info!("config.toml удалён после импорта в БД");

                config
            } else {
                Self::default_whisper()
            }
        } else {
            Self::default_whisper()
        };

        // Сохраняем в БД
        config.save()?;
        tracing::info!("Конфигурация сохранена в БД");

        Ok(config)
    }

    /// Сохраняет конфигурацию в SQLite БД
    pub fn save(&self) -> Result<(), ArcanaError> {
        let db_path = Self::history_db_path()
            .ok_or_else(|| ArcanaError::Config("Не удалось определить путь к БД".into()))?;
        let audio_cache = Self::audio_cache_dir()
            .ok_or_else(|| ArcanaError::Config("Не удалось определить путь к кэшу".into()))?;

        let db = crate::history::HistoryDB::new(&db_path, audio_cache)?;
        let json_str = serde_json::to_string(self)
            .map_err(|e| ArcanaError::Config(format!("Ошибка сериализации конфига: {}", e)))?;
        db.set_setting("core_config", &json_str)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_default_config_has_valid_values() {
        let config = CoreConfig::default();
        assert_eq!(config.transcriber, TranscriberType::Vosk);
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.max_record_secs, 20);
        assert!(config.auto_type);
        assert!(config.debug);
        assert_eq!(config.hotkey, "Super+W");
        assert!(config.model_path.ends_with("models/vosk-model-ru-0.42"));
        assert!(config.whisper_model_path.ends_with("models/ggml-large-v3-turbo.bin"));
        assert!(config.gigaam_model_path.ends_with("models/gigaam-v3-e2e-ctc"));
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let config = CoreConfig::default();
        let toml_str = toml::to_string_pretty(&config).expect("Сериализация не должна падать");
        let restored: CoreConfig = toml::from_str(&toml_str).expect("Десериализация не должна падать");

        assert_eq!(config.sample_rate, restored.sample_rate);
        assert_eq!(config.max_record_secs, restored.max_record_secs);
        assert_eq!(config.auto_type, restored.auto_type);
        assert_eq!(config.hotkey, restored.hotkey);
    }

    #[test]
    fn test_deserialize_partial_config() {
        // Проверяем, что частичный TOML (без всех полей) даёт ошибку
        let partial_toml = r#"
sample_rate = 16000
auto_type = false
"#;
        let result: Result<CoreConfig, _> = toml::from_str(partial_toml);
        // Должна быть ошибка, т.к. не все поля указаны
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_load_to_temp_file() {
        let dir = std::env::temp_dir().join("arcanaglyph_test_config");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("test_config.toml");

        let config = CoreConfig {
            transcriber: TranscriberType::Vosk,
            model_path: PathBuf::from("/tmp/test-model"),
            whisper_model_path: PathBuf::from("/tmp/test-whisper-model"),
            gigaam_model_path: PathBuf::from("/tmp/test-gigaam-model"),
            sample_rate: 16000,
            max_record_secs: 30,
            auto_type: false,
            hotkey: "Ctrl+Shift+R".to_string(),
            hotkey_pause: String::new(),
            debug: true,
            vad_enabled: true,
            vad_silence_secs: 7,
            remove_fillers: true,
            retention_hours: 0,
            start_minimized: false,
            preload_models: vec![],
        };

        let content = toml::to_string_pretty(&config).unwrap();
        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();

        let loaded_content = std::fs::read_to_string(&file_path).unwrap();
        let loaded: CoreConfig = toml::from_str(&loaded_content).unwrap();

        assert_eq!(loaded.sample_rate, 16000);
        assert_eq!(loaded.max_record_secs, 30);
        assert!(!loaded.auto_type);
        assert_eq!(loaded.hotkey, "Ctrl+Shift+R");
        assert_eq!(loaded.model_path, PathBuf::from("/tmp/test-model"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_path_returns_some() {
        // На большинстве систем config_path должен вернуть Some
        let path = CoreConfig::config_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.ends_with("config.toml"));
    }
}
