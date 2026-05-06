// crates/arcanaglyph-core/src/config.rs

use crate::error::ArcanaError;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Движок транскрибации.
///
/// Все варианты присутствуют всегда — это нужно, чтобы persisted JSON в SQLite
/// не ломался у пользователей, которые ранее выбирали Vosk/Whisper, при последующей
/// сборке без соответствующего cargo feature. Доступность движка в текущей сборке
/// проверяется через [`TranscriberType::is_compiled_in`].
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TranscriberType {
    /// Vosk — быстрый, потоковый, менее точный
    Vosk,
    /// Whisper — медленнее, значительно точнее
    Whisper,
    /// GigaAM v3 — лучший для русского (ONNX, SberDevices). Дефолтный движок.
    #[default]
    GigaAm,
    /// Qwen3-ASR — мультиязычный (ONNX, Alibaba)
    Qwen3Asr,
}

impl TranscriberType {
    /// Включён ли этот движок в текущую сборку через cargo feature.
    /// GigaAM считается включённым при ЛЮБОМ из двух ort-backend'ов (`gigaam` с
    /// download-binaries или `gigaam-system-ort` с load-dynamic) — различие в способе
    /// доставки libonnxruntime для UI не важно, важен только сам движок GigaAM.
    pub const fn is_compiled_in(&self) -> bool {
        match self {
            Self::Vosk => cfg!(feature = "vosk"),
            Self::Whisper => cfg!(feature = "whisper"),
            Self::GigaAm => cfg!(feature = "gigaam") || cfg!(feature = "gigaam-system-ort"),
            Self::Qwen3Asr => cfg!(feature = "qwen3asr"),
        }
    }

    /// Список движков, скомпилированных в текущую сборку.
    /// Используется фронтендом для отрисовки disabled-пунктов в dropdown'е.
    pub fn compiled_engines() -> Vec<TranscriberType> {
        [Self::Vosk, Self::Whisper, Self::GigaAm, Self::Qwen3Asr]
            .into_iter()
            .filter(Self::is_compiled_in)
            .collect()
    }

    /// Строковое представление для UI / persistence.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Vosk => "vosk",
            Self::Whisper => "whisper",
            Self::GigaAm => "gigaam",
            Self::Qwen3Asr => "qwen3asr",
        }
    }
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
    /// Путь к директории Qwen3-ASR (для transcriber = "qwen3asr")
    /// Директория: onnx_models/ (4 onnx файла + embed_tokens.bin) + tokenizer.json
    #[serde(default = "default_qwen3asr_model_path")]
    pub qwen3asr_model_path: PathBuf,
    /// Авто-стоп записи при тишине после речи
    #[serde(default = "default_true")]
    pub vad_enabled: bool,
    /// Секунды тишины после речи для авто-стопа (если vad_enabled)
    #[serde(default = "default_vad_silence_secs")]
    pub vad_silence_secs: u64,
    /// Удалять слова-паразиты из транскрибации (э, э-э, ээ, эм, мм)
    #[serde(default = "default_true")]
    pub remove_fillers: bool,
    /// Программное усиление микрофона (fallback для устройств без override).
    /// 1.0 = без усиления, 2.0 = +6 дБ, 5.0 = +14 дБ.
    /// Применяется к сэмплам с saturation (clip на ±32767). Работает на любой ОС.
    #[serde(default = "default_mic_gain")]
    pub mic_gain: f32,
    /// Per-device override для усиления микрофона.
    /// Ключ — имя устройства как возвращает `cpal::Device::name()` (например "default",
    /// "Anker SoundCore Headset Mono", "HDA Intel PCH ALC269VC Analog Mono").
    /// Значение — gain для этого устройства. Если устройства нет в map — берётся
    /// глобальный `mic_gain` (выше). Пользователь настраивает gain для текущего
    /// активного микрофона; смена мика в системе → подхватывается соответствующий gain.
    #[serde(default)]
    pub mic_gain_per_device: HashMap<String, f32>,
    /// Срок хранения записей в часах (0 = хранить вечно)
    #[serde(default = "default_retention_hours")]
    pub retention_hours: u64,
    /// Автозапуск при входе в систему
    #[serde(default)]
    pub autostart: bool,
    /// Запускать в свёрнутом виде (сразу в трей)
    #[serde(default)]
    pub start_minimized: bool,
    /// Модели для предзагрузки при старте (помимо основной)
    #[serde(default)]
    pub preload_models: Vec<TranscriberType>,
    /// Показывать плавающий виджет записи поверх всех окон
    #[serde(default = "default_true")]
    pub show_widget: bool,
    /// Логическая позиция виджета на экране: top-left/top-center/top-right,
    /// middle-left/middle-center/middle-right, bottom-left/bottom-center/bottom-right.
    /// Любое невалидное значение трактуется как `bottom-center`. На Wayland mutter
    /// может проигнорировать выбор — это ожидаемое поведение протокола.
    #[serde(default = "default_widget_position")]
    pub widget_position: String,
    /// Показывать иконку в системном трее
    #[serde(default = "default_true")]
    pub show_tray: bool,
    /// Базовый путь к директории моделей
    #[serde(default = "default_models_dir")]
    pub models_base_dir: PathBuf,
    /// Выбранный пользователем период фильтра на странице истории (секунды, 0 = все записи)
    #[serde(default = "default_history_filter_secs")]
    pub history_filter_secs: u64,
    /// Язык интерфейса: "ru" или "en" (пустая строка = авто по локали системы)
    #[serde(default)]
    pub language: String,
}

fn default_models_dir() -> PathBuf {
    CoreConfig::models_dir().unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("models")
    })
}

fn default_whisper_model_path() -> PathBuf {
    default_models_dir().join("ggml-large-v3-turbo.bin")
}

fn default_qwen3asr_model_path() -> PathBuf {
    default_models_dir().join("qwen3-asr-0.6b")
}

fn default_gigaam_model_path() -> PathBuf {
    default_models_dir().join("gigaam-v3-e2e-ctc")
}

fn default_true() -> bool {
    true
}

fn default_vad_silence_secs() -> u64 {
    7
}

fn default_retention_hours() -> u64 {
    24
}

fn default_history_filter_secs() -> u64 {
    86400
}

fn default_mic_gain() -> f32 {
    1.0
}

fn default_widget_position() -> String {
    "bottom-center".to_string()
}

/// Вычисляет (x, y) позиции виджета записи на экране по логическому имени.
///
/// `screen_w`/`screen_h` и `widget_w`/`widget_h` — в логических пикселях
/// (т.е. уже поделены на scale_factor). MARGIN/TOP_OFFSET/BOTTOM_OFFSET подобраны
/// эмпирически: 24px от боковых краёв (визуально не «впритык»), 48px от верха
/// (учитывает GNOME top-bar), 60px от низа (учитывает GNOME-Shell dock / taskbar).
/// Любое невалидное значение `pos` → fallback на bottom-center.
pub fn widget_position_xy(pos: &str, screen_w: f64, screen_h: f64, widget_w: f64, widget_h: f64) -> (f64, f64) {
    const MARGIN: f64 = 24.0;
    const TOP_OFFSET: f64 = 48.0;
    const BOTTOM_OFFSET: f64 = 60.0;
    let x_left = MARGIN;
    let x_center = (screen_w - widget_w) / 2.0;
    let x_right = screen_w - widget_w - MARGIN;
    let y_top = TOP_OFFSET;
    let y_mid = (screen_h - widget_h) / 2.0;
    let y_bot = screen_h - widget_h - BOTTOM_OFFSET;
    match pos {
        "top-left" => (x_left, y_top),
        "top-center" => (x_center, y_top),
        "top-right" => (x_right, y_top),
        "middle-left" => (x_left, y_mid),
        "middle-center" => (x_center, y_mid),
        "middle-right" => (x_right, y_mid),
        "bottom-left" => (x_left, y_bot),
        "bottom-right" => (x_right, y_bot),
        _ => (x_center, y_bot),
    }
}

impl Default for CoreConfig {
    fn default() -> Self {
        let models = default_models_dir();
        let model_path = models.join("vosk-model-ru-0.42");
        let whisper_model_path = models.join("ggml-large-v3-turbo.bin");
        let qwen3asr_model_path = models.join("qwen3-asr-0.6b");
        let gigaam_model_path = models.join("gigaam-v3-e2e-ctc");

        Self {
            transcriber: TranscriberType::Vosk,
            model_path,
            whisper_model_path,
            gigaam_model_path,
            qwen3asr_model_path,
            sample_rate: 48000,
            max_record_secs: 20,
            auto_type: true,
            hotkey: "Control+`".to_string(),
            hotkey_pause: "Control+Shift+`".to_string(),
            debug: false,
            vad_enabled: true,
            vad_silence_secs: 7,
            remove_fillers: true,
            mic_gain: 1.0,
            mic_gain_per_device: HashMap::new(),
            retention_hours: 24,
            autostart: false,
            start_minimized: false,
            preload_models: vec![],
            show_widget: true,
            widget_position: "bottom-center".to_string(),
            show_tray: true,
            models_base_dir: models,
            history_filter_secs: 86400,
            language: String::new(),
        }
    }
}

impl CoreConfig {
    /// Возвращает effective mic_gain для конкретного устройства.
    /// Если в `mic_gain_per_device` есть override для `device_name` — он используется,
    /// иначе возвращается глобальный `mic_gain`. Это позволяет настроить разное
    /// усиление для встроенного мика и подключаемых наушников.
    pub fn effective_gain(&self, device_name: &str) -> f32 {
        self.mic_gain_per_device
            .get(device_name)
            .copied()
            .unwrap_or(self.mic_gain)
    }

    /// Дефолтный конфиг с GigaAM по умолчанию (для новых пользователей)
    pub fn default_gigaam() -> Self {
        Self {
            transcriber: TranscriberType::GigaAm,
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

    /// Директория моделей: ~/.local/share/arcanaglyph/models/
    pub fn models_dir() -> Option<PathBuf> {
        ProjectDirs::from("com", "arcanaglyph", "ArcanaGlyph").map(|dirs| dirs.data_dir().join("models"))
    }

    /// Директория скриптов: ~/.config/arcanaglyph/scripts/
    pub fn scripts_dir() -> Option<PathBuf> {
        ProjectDirs::from("com", "arcanaglyph", "ArcanaGlyph").map(|dirs| dirs.config_dir().join("scripts"))
    }

    /// Название текущей модели (для записи в историю)
    pub fn transcriber_model_name(&self) -> String {
        match self.transcriber {
            TranscriberType::Vosk => self
                .model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "vosk".to_string()),
            TranscriberType::Whisper => self
                .whisper_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "whisper".to_string()),
            TranscriberType::GigaAm => self
                .gigaam_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "gigaam".to_string()),
            TranscriberType::Qwen3Asr => self
                .qwen3asr_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "qwen3asr".to_string()),
        }
    }

    /// Тип транскрайбера как строка
    pub fn transcriber_type_str(&self) -> String {
        match self.transcriber {
            TranscriberType::Vosk => "vosk".to_string(),
            TranscriberType::Whisper => "whisper".to_string(),
            TranscriberType::GigaAm => "gigaam".to_string(),
            TranscriberType::Qwen3Asr => "qwen3asr".to_string(),
        }
    }

    /// Загружает конфигурацию из SQLite БД. При первом запуске импортирует из config.toml если есть.
    pub fn load() -> Result<Self, ArcanaError> {
        let db_path =
            Self::history_db_path().ok_or_else(|| ArcanaError::Config("Не удалось определить путь к БД".into()))?;
        let audio_cache =
            Self::audio_cache_dir().ok_or_else(|| ArcanaError::Config("Не удалось определить путь к кэшу".into()))?;

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
                let content = std::fs::read_to_string(&config_path).map_err(|e| {
                    ArcanaError::Config(format!("Не удалось прочитать {}: {}", config_path.display(), e))
                })?;
                let config: CoreConfig = toml::from_str(&content)
                    .map_err(|e| ArcanaError::Config(format!("Ошибка парсинга config.toml: {}", e)))?;

                tracing::info!("Импорт настроек из config.toml");
                let _ = std::fs::remove_file(&config_path);
                tracing::info!("config.toml удалён после импорта в БД");

                config
            } else {
                Self::default_gigaam()
            }
        } else {
            Self::default_gigaam()
        };

        // Сохраняем в БД
        config.save()?;
        tracing::info!("Конфигурация сохранена в БД");

        Ok(config)
    }

    /// Сохраняет конфигурацию в SQLite БД
    pub fn save(&self) -> Result<(), ArcanaError> {
        let db_path =
            Self::history_db_path().ok_or_else(|| ArcanaError::Config("Не удалось определить путь к БД".into()))?;
        let audio_cache =
            Self::audio_cache_dir().ok_or_else(|| ArcanaError::Config("Не удалось определить путь к кэшу".into()))?;

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
        assert!(!config.debug);
        assert_eq!(config.hotkey, "Control+`");
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
            qwen3asr_model_path: PathBuf::from("/tmp/test-qwen3asr-model"),
            sample_rate: 16000,
            max_record_secs: 30,
            auto_type: false,
            hotkey: "Ctrl+Shift+R".to_string(),
            hotkey_pause: String::new(),
            debug: false,
            vad_enabled: true,
            vad_silence_secs: 7,
            remove_fillers: true,
            mic_gain: 1.0,
            mic_gain_per_device: HashMap::new(),
            retention_hours: 24,
            autostart: false,
            start_minimized: false,
            preload_models: vec![],
            show_widget: true,
            widget_position: "bottom-center".to_string(),
            show_tray: true,
            models_base_dir: PathBuf::from("/tmp/test-models"),
            history_filter_secs: 86400,
            language: String::new(),
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
