-- v001: Начальная схема БД
-- Таблицы для хранения аудиозаписей и результатов транскрибации

CREATE TABLE recordings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    audio_path TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    duration_secs INTEGER NOT NULL
);

CREATE TABLE transcriptions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recording_id INTEGER NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
    text TEXT NOT NULL,
    model_name TEXT NOT NULL,
    transcriber_type TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_rec_timestamp ON recordings(timestamp DESC);
CREATE INDEX idx_trans_recording ON transcriptions(recording_id);
