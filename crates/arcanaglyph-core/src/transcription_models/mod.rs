// crates/arcanaglyph-core/src/transcription_models/mod.rs
//
// Реестр доступных моделей для распознавания речи.
// Каждый файл описывает одну модель: тип, имя, путь, размер, URL скачивания.
// Добавление новой модели: создать файл *_speech_model.rs и добавить в all().

// Модули с метаданными моделей (статические `SpeechModelInfo`) компилируются всегда —
// они нужны UI, чтобы показать пользователю даже те модели, чей backend в текущей
// сборке не включён (метка «не доступно»). Сами transcriber'ы по-прежнему за
// `#[cfg(feature = ...)]` и в реестр `all()` попадают только активные.
pub mod gigaam_v3_fp32_speech_model;
pub mod gigaam_v3_speech_model;
pub mod qwen3_asr_speech_model;
pub mod vosk_russian_speech_model;
pub mod whisper_large_v3_turbo_speech_model;
pub mod whisper_tiny_speech_model;

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
    /// Ожидаемый минимальный размер главного файла в байтах.
    /// Используется для валидации целостности после скачивания и при старте приложения:
    /// если файл существует, но меньше порога — он считается повреждённым (прерванное
    /// скачивание) и перекачивается заново. Оптимально брать ~90% реального размера.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_min_size_bytes: Option<u64>,
}

/// Все доступные модели распознавания речи (только для движков, включённых в сборку).
#[allow(clippy::vec_init_then_push, unused_mut)]
pub fn all() -> Vec<&'static SpeechModelInfo> {
    all_with_availability()
        .into_iter()
        .filter_map(|(m, available)| if available { Some(m) } else { None })
        .collect()
}

/// Все модели, известные приложению, с признаком «есть ли backend в этой сборке».
/// UI использует это, чтобы показывать карточки даже для отсутствующих движков
/// (с пометкой «не доступно»), а пользователь видел, что в принципе поддерживается.
pub fn all_with_availability() -> Vec<(&'static SpeechModelInfo, bool)> {
    vec![
        (&vosk_russian_speech_model::MODEL, cfg!(feature = "vosk")),
        // Whisper: Tiny идёт первой (быстрая, ~80 МБ — для слабых CPU без AVX2).
        // Large V3 Turbo — точнее, но очень медленно на безAVX2 (~20× от AVX2-machine).
        // UI-dropdown "Движок транскрибации" должен предлагать оба варианта раздельно.
        (&whisper_tiny_speech_model::MODEL, cfg!(feature = "whisper")),
        (&whisper_large_v3_turbo_speech_model::MODEL, cfg!(feature = "whisper")),
        // GigaAM. INT8 — для ort-backend'ов (`gigaam`, `gigaam-system-ort`),
        // FP32 — только для tract'а (`gigaam-tract`, экспериментально).
        // Оба имеют transcriber_type = "gigaam".
        (
            &gigaam_v3_speech_model::MODEL,
            cfg!(any(feature = "gigaam", feature = "gigaam-system-ort")),
        ),
        (&gigaam_v3_fp32_speech_model::MODEL, cfg!(feature = "gigaam-tract")),
        (&qwen3_asr_speech_model::MODEL, cfg!(feature = "qwen3asr")),
    ]
}

/// Найти модель по идентификатору
pub fn find(id: &str) -> Option<&'static SpeechModelInfo> {
    all().into_iter().find(|m| m.id == id)
}

/// Найти первую модель для заданного типа транскрайбера ("vosk", "whisper", "gigaam", "qwen3asr").
/// Возвращает None если соответствующая cargo-feature не включена в сборку.
pub fn find_by_transcriber_type(t_type: &str) -> Option<&'static SpeechModelInfo> {
    all().into_iter().find(|m| m.transcriber_type == t_type)
}

/// Найти модель по типу транскрайбера И имени файла (`default_filename`).
/// Если точного совпадения нет — возвращает первую модель этого типа.
pub fn find_by_type_and_filename(t_type: &str, filename: &str) -> Option<&'static SpeechModelInfo> {
    let models = all();
    models
        .iter()
        .find(|m| m.transcriber_type == t_type && m.default_filename == filename)
        .copied()
        .or_else(|| models.into_iter().find(|m| m.transcriber_type == t_type))
}
