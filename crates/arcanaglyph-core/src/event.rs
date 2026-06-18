// crates/arcanaglyph-core/src/event.rs
//
// Тип события движка, рассылаемого подписчикам через broadcast-канал. Вынесен из
// `engine` в нейтральный модуль, чтобы и `engine`, и `audio` (который шлёт события
// во время записи) зависели от него, не образуя цикла `audio ↔ engine`.

/// События движка, рассылаемые подписчикам
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Запись началась
    RecordingStarted,
    /// Запись приостановлена
    RecordingPaused,
    /// Запись возобновлена
    RecordingResumed,
    /// Результат транскрибации
    TranscriptionResult(String),
    /// Транскрибация началась (запись завершена, идёт распознавание)
    Transcribing,
    /// Обработка завершена, система готова к новой записи
    FinishedProcessing,
    /// Запрос на вывод окна на передний план (когда окно видимо)
    RequestFocus,
    /// Начата загрузка модели в память (eager-preload из save_config или lazy-fallback в trigger).
    /// Payload — отображаемое имя модели для UI ("Vosk Russian 0.42").
    ModelLoading(String),
    /// Модель загружена, приложение готово к работе
    ModelLoaded,
    /// Ошибка, которую нужно показать пользователю
    Error(String),
}
