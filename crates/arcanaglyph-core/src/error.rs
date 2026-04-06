// crates/arcanaglyph-core/src/error.rs

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
}
