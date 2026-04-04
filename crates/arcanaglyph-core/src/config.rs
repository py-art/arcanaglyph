// crates/arcanaglyph-core/src/config.rs

use crate::error::ArcanaError;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Конфигурация ядра ArcanaGlyph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    /// Путь к Vosk-модели
    pub model_path: PathBuf,
    /// Частота дискретизации аудио (Гц)
    pub sample_rate: u32,
    /// Таймаут тишины (секунды): если нет новых слов столько времени — запись автоматически останавливается
    pub max_record_secs: u64,
    /// Автоматически вставлять распознанный текст в активное окно
    pub auto_type: bool,
    /// Горячая клавиша для триггера (формат Tauri: "Super+Alt+Control+Space")
    pub hotkey: String,
    /// Режим отладки: выводить промежуточные результаты распознавания в терминал
    pub debug: bool,
}

impl Default for CoreConfig {
    fn default() -> Self {
        // Пытаемся найти модель относительно текущей директории
        let model_path = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("models/vosk-model-ru-0.42");

        Self {
            model_path,
            sample_rate: 48000,
            max_record_secs: 20,
            auto_type: true,
            hotkey: "Super+Alt+Control+Space".to_string(),
            debug: true,
        }
    }
}

impl CoreConfig {
    /// Путь к конфигурационному файлу: ~/.config/arcanaglyph/config.toml
    pub fn config_path() -> Option<PathBuf> {
        ProjectDirs::from("com", "arcanaglyph", "ArcanaGlyph").map(|dirs| dirs.config_dir().join("config.toml"))
    }

    /// Загружает конфигурацию из файла. Если файл не существует — создаёт с дефолтными значениями.
    pub fn load() -> Result<Self, ArcanaError> {
        let config_path = Self::config_path()
            .ok_or_else(|| ArcanaError::Config("Не удалось определить директорию конфигурации".into()))?;

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .map_err(|e| ArcanaError::Config(format!("Не удалось прочитать {}: {}", config_path.display(), e)))?;

            let config: CoreConfig =
                toml::from_str(&content).map_err(|e| ArcanaError::Config(format!("Ошибка парсинга конфига: {}", e)))?;

            tracing::info!("Конфигурация загружена из {}", config_path.display());
            Ok(config)
        } else {
            let config = Self::default();
            // Создаём файл с дефолтными значениями
            if let Err(e) = config.save() {
                tracing::warn!("Не удалось сохранить дефолтный конфиг: {}", e);
            }
            tracing::info!("Создан дефолтный конфиг: {}", config_path.display());
            Ok(config)
        }
    }

    /// Сохраняет конфигурацию в файл
    pub fn save(&self) -> Result<(), ArcanaError> {
        let config_path = Self::config_path()
            .ok_or_else(|| ArcanaError::Config("Не удалось определить директорию конфигурации".into()))?;

        // Создаём директорию, если не существует
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ArcanaError::Config(format!("Не удалось создать директорию конфигурации: {}", e)))?;
        }

        let content = toml::to_string_pretty(self)
            .map_err(|e| ArcanaError::Config(format!("Ошибка сериализации конфига: {}", e)))?;

        std::fs::write(&config_path, content)
            .map_err(|e| ArcanaError::Config(format!("Не удалось записать {}: {}", config_path.display(), e)))?;

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
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.max_record_secs, 20);
        assert!(config.auto_type);
        assert!(config.debug);
        assert_eq!(config.hotkey, "Super+Alt+Control+Space");
        assert!(config.model_path.ends_with("models/vosk-model-ru-0.42"));
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
            model_path: PathBuf::from("/tmp/test-model"),
            sample_rate: 16000,
            max_record_secs: 30,
            auto_type: false,
            hotkey: "Ctrl+Shift+R".to_string(),
            debug: true,
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
