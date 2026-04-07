// crates/arcanaglyph-core/src/transcription_models/mod.rs
//
// Реестр доступных моделей для распознавания речи.
// Каждый файл описывает одну модель: тип, имя, путь, размер, URL скачивания.
// Добавление новой модели: создать файл *_speech_model.rs и добавить в all().

pub mod gigaam_v3_speech_model;
pub mod qwen3_asr_speech_model;
pub mod vosk_russian_speech_model;
pub mod whisper_large_v3_turbo_speech_model;

use serde::Serialize;

/// Описание модели распознавания речи
#[derive(Debug, Clone, Serialize)]
pub struct SpeechModelInfo {
    /// Уникальный идентификатор модели
    pub id: &'static str,
    /// Отображаемое имя в UI
    pub display_name: &'static str,
    /// Тип транскрайбера: "vosk" или "whisper"
    pub transcriber_type: &'static str,
    /// Имя файла/директории модели по умолчанию
    pub default_filename: &'static str,
    /// Описание модели
    pub description: &'static str,
    /// Примерный размер модели
    pub size: &'static str,
    /// URL для скачивания (основной или первый файл)
    pub download_url: &'static str,
    /// Дополнительные файлы для скачивания (URL → относительный путь внутри директории модели)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_files: Option<&'static [(&'static str, &'static str)]>,
}

/// Все доступные модели распознавания речи
pub fn all() -> Vec<&'static SpeechModelInfo> {
    vec![
        &vosk_russian_speech_model::MODEL,
        &whisper_large_v3_turbo_speech_model::MODEL,
        &gigaam_v3_speech_model::MODEL,
        &qwen3_asr_speech_model::MODEL,
    ]
}

/// Найти модель по идентификатору
pub fn find(id: &str) -> Option<&'static SpeechModelInfo> {
    all().into_iter().find(|m| m.id == id)
}
