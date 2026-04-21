// crates/arcanaglyph-core/src/history.rs

use crate::error::ArcanaError;
use rusqlite::Connection;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Запись аудио
#[derive(Debug, Clone, Serialize)]
pub struct Recording {
    pub id: i64,
    pub audio_path: String,
    pub timestamp: i64,
    pub duration_secs: u32,
}

/// Результат транскрибации
#[derive(Debug, Clone, Serialize)]
pub struct Transcription {
    pub id: i64,
    pub recording_id: i64,
    pub text: String,
    pub model_name: String,
    pub transcriber_type: String,
    pub created_at: i64,
}

/// Запись истории с транскрибациями
#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub recording: Recording,
    pub transcriptions: Vec<Transcription>,
    pub audio_exists: bool,
}

/// База данных истории транскрибаций
pub struct HistoryDB {
    conn: Mutex<Connection>,
    audio_cache_dir: PathBuf,
}

impl HistoryDB {
    /// Создаёт или открывает БД, применяет миграции
    pub fn new(db_path: &Path, audio_cache_dir: PathBuf) -> Result<Self, ArcanaError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ArcanaError::Database(format!("Не удалось создать директорию БД: {}", e)))?;
        }
        std::fs::create_dir_all(&audio_cache_dir)
            .map_err(|e| ArcanaError::Database(format!("Не удалось создать директорию кэша аудио: {}", e)))?;

        let conn =
            Connection::open(db_path).map_err(|e| ArcanaError::Database(format!("Не удалось открыть БД: {}", e)))?;

        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| ArcanaError::Database(format!("Не удалось включить foreign keys: {}", e)))?;

        // Применяем миграции
        crate::db::run_migrations(&conn)?;

        tracing::info!("БД истории открыта: {:?}", db_path);
        Ok(Self {
            conn: Mutex::new(conn),
            audio_cache_dir,
        })
    }

    /// Добавляет запись аудио
    /// Получить настройку по ключу
    pub fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            rusqlite::params![key],
            |row| row.get(0),
        )
        .ok()
    }

    /// Установить настройку
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), ArcanaError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ArcanaError::Database(format!("Mutex: {}", e)))?;
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )
        .map_err(|e| ArcanaError::Database(format!("Ошибка записи настройки: {}", e)))?;
        Ok(())
    }

    /// Получить все настройки
    pub fn get_all_settings(&self) -> Result<std::collections::HashMap<String, String>, ArcanaError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ArcanaError::Database(format!("Mutex: {}", e)))?;
        let mut stmt = conn
            .prepare("SELECT key, value FROM settings")
            .map_err(|e| ArcanaError::Database(format!("Ошибка запроса настроек: {}", e)))?;
        let result = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| ArcanaError::Database(format!("Ошибка маппинга: {}", e)))?
            .collect::<Result<std::collections::HashMap<_, _>, _>>()
            .map_err(|e| ArcanaError::Database(format!("Ошибка сбора: {}", e)))?;
        Ok(result)
    }

    pub fn add_recording(&self, audio_path: &str, duration_secs: u32) -> Result<i64, ArcanaError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ArcanaError::Database(format!("Mutex: {}", e)))?;
        let timestamp = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO recordings (audio_path, timestamp, duration_secs) VALUES (?1, ?2, ?3)",
            rusqlite::params![audio_path, timestamp, duration_secs],
        )
        .map_err(|e| ArcanaError::Database(format!("Ошибка добавления записи: {}", e)))?;
        Ok(conn.last_insert_rowid())
    }

    /// Добавляет результат транскрибации
    pub fn add_transcription(
        &self,
        recording_id: i64,
        text: &str,
        model_name: &str,
        transcriber_type: &str,
    ) -> Result<i64, ArcanaError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ArcanaError::Database(format!("Mutex: {}", e)))?;
        let created_at = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO transcriptions (recording_id, text, model_name, transcriber_type, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![recording_id, text, model_name, transcriber_type, created_at],
        )
        .map_err(|e| ArcanaError::Database(format!("Ошибка добавления транскрибации: {}", e)))?;
        Ok(conn.last_insert_rowid())
    }

    /// Запрос истории с пагинацией и фильтром по времени
    pub fn query(
        &self,
        since_timestamp: i64,
        limit: u32,
        offset: u32,
    ) -> Result<(Vec<HistoryEntry>, u32), ArcanaError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ArcanaError::Database(format!("Mutex: {}", e)))?;

        // Общее количество записей
        let total: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM recordings WHERE timestamp >= ?1",
                rusqlite::params![since_timestamp],
                |row| row.get(0),
            )
            .map_err(|e| ArcanaError::Database(format!("Ошибка подсчёта: {}", e)))?;

        // Записи с пагинацией
        let mut stmt = conn
            .prepare(
                "SELECT id, audio_path, timestamp, duration_secs FROM recordings
                 WHERE timestamp >= ?1 ORDER BY timestamp DESC LIMIT ?2 OFFSET ?3",
            )
            .map_err(|e| ArcanaError::Database(format!("Ошибка запроса: {}", e)))?;

        let recordings: Vec<Recording> = stmt
            .query_map(rusqlite::params![since_timestamp, limit, offset], |row| {
                Ok(Recording {
                    id: row.get(0)?,
                    audio_path: row.get(1)?,
                    timestamp: row.get(2)?,
                    duration_secs: row.get(3)?,
                })
            })
            .map_err(|e| ArcanaError::Database(format!("Ошибка маппинга: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ArcanaError::Database(format!("Ошибка сбора: {}", e)))?;

        // Для каждой записи — загрузить транскрибации
        let mut entries = Vec::with_capacity(recordings.len());
        let mut trans_stmt = conn
            .prepare(
                "SELECT id, recording_id, text, model_name, transcriber_type, created_at
                 FROM transcriptions WHERE recording_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| ArcanaError::Database(format!("Ошибка запроса транскрибаций: {}", e)))?;

        for rec in recordings {
            let transcriptions: Vec<Transcription> = trans_stmt
                .query_map(rusqlite::params![rec.id], |row| {
                    Ok(Transcription {
                        id: row.get(0)?,
                        recording_id: row.get(1)?,
                        text: row.get(2)?,
                        model_name: row.get(3)?,
                        transcriber_type: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })
                .map_err(|e| ArcanaError::Database(format!("Ошибка маппинга транскрибаций: {}", e)))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ArcanaError::Database(format!("Ошибка сбора транскрибаций: {}", e)))?;

            let audio_exists = Path::new(&rec.audio_path).exists();
            entries.push(HistoryEntry {
                recording: rec,
                transcriptions,
                audio_exists,
            });
        }

        Ok((entries, total))
    }

    /// Получить транскрибации для конкретной записи
    pub fn get_transcriptions(&self, recording_id: i64) -> Result<Vec<Transcription>, ArcanaError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ArcanaError::Database(format!("Mutex: {}", e)))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, recording_id, text, model_name, transcriber_type, created_at
                 FROM transcriptions WHERE recording_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| ArcanaError::Database(format!("Ошибка запроса: {}", e)))?;

        let result = stmt
            .query_map(rusqlite::params![recording_id], |row| {
                Ok(Transcription {
                    id: row.get(0)?,
                    recording_id: row.get(1)?,
                    text: row.get(2)?,
                    model_name: row.get(3)?,
                    transcriber_type: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| ArcanaError::Database(format!("Ошибка маппинга: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ArcanaError::Database(format!("Ошибка сбора: {}", e)))?;

        Ok(result)
    }

    /// Удаляет запись и её транскрибации + аудиофайл
    pub fn delete_recording(&self, id: i64) -> Result<(), ArcanaError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ArcanaError::Database(format!("Mutex: {}", e)))?;

        // Получаем путь к аудио для удаления файла
        let audio_path: Option<String> = conn
            .query_row(
                "SELECT audio_path FROM recordings WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .ok();

        // Каскадно удаляет транскрибации (foreign key ON DELETE CASCADE)
        conn.execute("DELETE FROM recordings WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| ArcanaError::Database(format!("Ошибка удаления: {}", e)))?;

        // Удаляем аудиофайл если существует
        if let Some(path) = audio_path {
            let _ = std::fs::remove_file(&path);
        }

        Ok(())
    }

    /// Очищает всю историю и кэш аудио
    pub fn clear(&self) -> Result<(), ArcanaError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| ArcanaError::Database(format!("Mutex: {}", e)))?;
        conn.execute_batch("DELETE FROM transcriptions; DELETE FROM recordings;")
            .map_err(|e| ArcanaError::Database(format!("Ошибка очистки: {}", e)))?;

        // Чистим директорию кэша
        if self.audio_cache_dir.exists() {
            let _ = std::fs::remove_dir_all(&self.audio_cache_dir);
            let _ = std::fs::create_dir_all(&self.audio_cache_dir);
        }

        tracing::info!("История очищена");
        Ok(())
    }

    /// Удаляет записи старше `hours` часов (и их аудиофайлы)
    pub fn cleanup_old_recordings(&self, hours: u64) -> Result<u64, ArcanaError> {
        if hours == 0 {
            return Ok(0);
        }
        let cutoff = chrono::Utc::now().timestamp() - (hours as i64 * 3600);
        let conn = self
            .conn
            .lock()
            .map_err(|e| ArcanaError::Database(format!("Mutex: {}", e)))?;

        // Собираем пути аудио для удаления файлов
        let mut stmt = conn
            .prepare("SELECT id, audio_path FROM recordings WHERE timestamp < ?1")
            .map_err(|e| ArcanaError::Database(format!("Ошибка подготовки запроса: {}", e)))?;
        let rows: Vec<(i64, String)> = stmt
            .query_map(rusqlite::params![cutoff], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| ArcanaError::Database(format!("Ошибка выборки: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        let count = rows.len() as u64;

        // Удаляем аудиофайлы
        for (_, path) in &rows {
            let _ = std::fs::remove_file(path);
        }

        // Удаляем записи из БД (каскадно удалит транскрибации)
        conn.execute("DELETE FROM recordings WHERE timestamp < ?1", rusqlite::params![cutoff])
            .map_err(|e| ArcanaError::Database(format!("Ошибка удаления: {}", e)))?;

        if count > 0 {
            tracing::info!("Автоочистка: удалено {} записей старше {} ч.", count, hours);
        }
        Ok(count)
    }

    /// Проверяет физическое наличие аудиофайла
    pub fn audio_exists(&self, recording_id: i64) -> bool {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return false,
        };
        let path: Option<String> = conn
            .query_row(
                "SELECT audio_path FROM recordings WHERE id = ?1",
                rusqlite::params![recording_id],
                |row| row.get(0),
            )
            .ok();
        path.is_some_and(|p| Path::new(&p).exists())
    }

    /// Путь к директории аудиокэша
    pub fn audio_cache_path(&self) -> &Path {
        &self.audio_cache_dir
    }

    /// Экспорт всей истории в текстовый формат (txt или csv)
    pub fn export(&self, format: &str) -> Result<String, ArcanaError> {
        let (entries, _) = self.query(0, u32::MAX, 0)?;

        match format {
            "csv" => {
                // BOM для корректного отображения UTF-8 в Excel
                let mut out = String::from("\u{FEFF}Дата;Длительность (сек);Модель;Движок;Текст\n");
                for entry in &entries {
                    for t in &entry.transcriptions {
                        let date = chrono::DateTime::from_timestamp(entry.recording.timestamp, 0)
                            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_default();
                        // Экранируем кавычки в тексте для CSV
                        let text = t.text.replace('"', "\"\"");
                        out.push_str(&format!(
                            "{};{};{};{};\"{}\"\n",
                            date, entry.recording.duration_secs, t.model_name, t.transcriber_type, text
                        ));
                    }
                }
                Ok(out)
            }
            _ => {
                // txt формат
                let mut out = String::new();
                for entry in &entries {
                    let date = chrono::DateTime::from_timestamp(entry.recording.timestamp, 0)
                        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_default();
                    out.push_str(&format!("[{}] ({}с)\n", date, entry.recording.duration_secs));
                    for t in &entry.transcriptions {
                        out.push_str(&format!("  [{}] {}\n", t.model_name, t.text));
                    }
                    out.push('\n');
                }
                Ok(out)
            }
        }
    }
}
