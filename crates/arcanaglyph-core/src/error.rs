// crates/arcanaglyph-core/src/error.rs

use serde::Serialize;
use thiserror::Error;

/// Ошибки ядра ArcanaGlyph
#[derive(Debug, Error)]
pub enum ArcanaError {
    #[error("Ошибка аудиоустройства: {0}")]
    AudioDevice(String),

    #[error("Ошибка аудиопотока: {0}")]
    AudioStream(String),

    #[error("Ошибка загрузки модели: {0}")]
    ModelLoad(String),

    #[error("Ошибка распознавателя: {0}")]
    Recognizer(String),

    #[error("Ошибка сети: {0}")]
    Network(String),

    #[error("Ошибка симуляции ввода: {0}")]
    InputSimulation(String),

    #[error("Ошибка базы данных: {0}")]
    Database(String),

    #[error("Ошибка конфигурации: {0}")]
    Config(String),

    #[error("Внутренняя ошибка: {0}")]
    Internal(String),

    #[error("Движок '{0}' не включён в эту сборку")]
    EngineNotAvailable(String),

    /// Транскрибация прервана пользователем (через `Transcriber::cancel()`).
    /// Не ошибка по сути; UI должен скрыть сообщение об ошибке и просто вернуться
    /// в idle-состояние.
    #[error("Транскрибация отменена")]
    Cancelled,
}

/// Сериализуемая ошибка для отправки во frontend через Tauri-events.
/// Отделена от внутреннего `ArcanaError`, чтобы:
///   - frontend получал стабильный JSON-контракт (`{ kind, message, hint? }`),
///     а не строку Display-форматтера, которая может меняться;
///   - можно было добавить `hint` с пользовательской подсказкой без рисков
///     загрязнить логи (`tracing::error!` логирует ArcanaError как есть);
///   - для одной `ModelLoad`-ветки внутри Rust можно выделить отдельные kind
///     для UI (например DiskSpace для «No space left on device»).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub kind: ApiErrorKind,
    pub message: String,
    /// Подсказка пользователю «что делать» (например «освободите место на диске»).
    /// `None` для технических ошибок (`Internal`) и для `Cancelled` — UI вообще
    /// не показывает тост для пользователя сам нажал «Стоп».
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Категория ошибки — стабильный enum для frontend-side маппинга в иконку/CTA.
/// `camelCase` для удобства JS-кода (`err.kind === 'audioDevice'`).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ApiErrorKind {
    AudioDevice,
    AudioStream,
    ModelLoad,
    /// Подкатегория `ModelLoad` для случая «No space left on device» — это самый
    /// частый failure mode на N5095 / маленьких SSD. Маппится из ArcanaError::ModelLoad
    /// через substring `"No space left"`. Грубо, но 95% Linux-кейсов покрывает.
    DiskSpace,
    Network,
    InputSimulation,
    EngineNotAvailable,
    Cancelled,
    Internal,
}

/// Платформо-зависимая подсказка для ошибок аудиоустройства. На Linux — pavucontrol,
/// на Windows/macOS — системные настройки звука (показывать «pavucontrol» на Windows
/// бессмысленно и сбивает с толку).
fn audio_device_hint() -> String {
    #[cfg(target_os = "windows")]
    {
        "Проверьте устройство записи: Параметры Windows → Система → Звук → Ввод; \
         и разрешите доступ к микрофону (Конфиденциальность → Микрофон)."
            .to_string()
    }
    #[cfg(target_os = "macos")]
    {
        "Проверьте микрофон: Системные настройки → Звук → Вход; и разрешите доступ \
         к микрофону в разделе Конфиденциальность."
            .to_string()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "Проверьте микрофон в pavucontrol → Input Devices.".to_string()
    }
}

impl ApiError {
    /// Создаёт ApiError из внутренней `ArcanaError`. Мост между core-типом и
    /// сериализуемым типом для UI. Hint выбирается per-variant — пользователю
    /// предлагается конкретное действие («проверьте микрофон в pavucontrol»,
    /// «освободите ≥ 2 ГБ» и т.п.).
    pub fn from_arcana(err: &ArcanaError) -> Self {
        match err {
            ArcanaError::AudioDevice(msg) => Self {
                kind: ApiErrorKind::AudioDevice,
                message: msg.clone(),
                hint: Some(audio_device_hint()),
            },
            ArcanaError::AudioStream(msg) => Self {
                kind: ApiErrorKind::AudioStream,
                message: msg.clone(),
                hint: Some("Закройте другие записывающие программы (Zoom, OBS, браузерные звонки).".into()),
            },
            ArcanaError::ModelLoad(msg) => {
                // Парсим текст ошибки. ORT/whisper.cpp возвращают «No space left on
                // device» и «expected N tensors, got M» как substring в нашем msg.
                if msg.contains("No space left") {
                    Self {
                        kind: ApiErrorKind::DiskSpace,
                        message: msg.clone(),
                        hint: Some("Освободите ≥ 2 ГБ на диске и повторите загрузку.".into()),
                    }
                } else if msg.contains("expected") && msg.contains("tensors") {
                    Self {
                        kind: ApiErrorKind::ModelLoad,
                        message: msg.clone(),
                        hint: Some("Файл модели повреждён. Удалите её в Settings → Models и скачайте заново.".into()),
                    }
                } else {
                    Self {
                        kind: ApiErrorKind::ModelLoad,
                        message: msg.clone(),
                        hint: Some("Не удалось загрузить модель. Проверьте логи в терминале.".into()),
                    }
                }
            }
            ArcanaError::Recognizer(msg) => Self {
                kind: ApiErrorKind::Internal,
                message: msg.clone(),
                hint: None,
            },
            ArcanaError::Network(msg) => Self {
                kind: ApiErrorKind::Network,
                message: msg.clone(),
                hint: Some("Проверьте интернет-соединение.".into()),
            },
            ArcanaError::InputSimulation(msg) => Self {
                kind: ApiErrorKind::InputSimulation,
                message: msg.clone(),
                hint: Some("На Wayland нажмите «Дать разрешение» в баннере.".into()),
            },
            ArcanaError::Database(msg) | ArcanaError::Config(msg) | ArcanaError::Internal(msg) => Self {
                kind: ApiErrorKind::Internal,
                message: msg.clone(),
                hint: None,
            },
            ArcanaError::EngineNotAvailable(engine) => Self {
                kind: ApiErrorKind::EngineNotAvailable,
                message: format!("Движок '{}' не включён в эту сборку", engine),
                hint: Some("Выберите другой движок в Settings → Engines.".into()),
            },
            ArcanaError::Cancelled => Self {
                kind: ApiErrorKind::Cancelled,
                message: "Транскрибация отменена".into(),
                hint: None,
            },
        }
    }

    /// Конверсия из произвольной строки ошибки — fallback когда у нас нет
    /// `ArcanaError` (например error был сконвертирован в String до того
    /// как мы дошли до `EngineEvent::Error`). Парсит префикс Display-форматтера
    /// `ArcanaError` (`«Ошибка аудиоустройства: ...»`, и т.п.) и выбирает kind.
    /// Не идеально, но покрывает существующий код где `e.to_string()` уже сделан.
    pub fn from_message(msg: &str) -> Self {
        // Парсим по префиксу Display-форматтера `ArcanaError`. Если строка пришла
        // не от ArcanaError (произвольный код) — попадает в `Internal`.
        if let Some(rest) = msg.strip_prefix("Ошибка аудиоустройства: ") {
            return Self::from_arcana(&ArcanaError::AudioDevice(rest.into()));
        }
        if let Some(rest) = msg.strip_prefix("Ошибка аудиопотока: ") {
            return Self::from_arcana(&ArcanaError::AudioStream(rest.into()));
        }
        if let Some(rest) = msg.strip_prefix("Ошибка загрузки модели: ") {
            return Self::from_arcana(&ArcanaError::ModelLoad(rest.into()));
        }
        if let Some(rest) = msg.strip_prefix("Ошибка сети: ") {
            return Self::from_arcana(&ArcanaError::Network(rest.into()));
        }
        if let Some(rest) = msg.strip_prefix("Ошибка симуляции ввода: ") {
            return Self::from_arcana(&ArcanaError::InputSimulation(rest.into()));
        }
        if msg == "Транскрибация отменена" {
            return Self::from_arcana(&ArcanaError::Cancelled);
        }
        // Микрофон не захватил речь / общие ошибки от audio::record_and_transcribe —
        // это тоже AudioDevice, но без префикса (приходит как просто string).
        if msg.contains("Микрофон") || msg.contains("микрофон") {
            return Self {
                kind: ApiErrorKind::AudioDevice,
                message: msg.into(),
                hint: Some(audio_device_hint()),
            };
        }
        Self {
            kind: ApiErrorKind::Internal,
            message: msg.into(),
            hint: None,
        }
    }
}
