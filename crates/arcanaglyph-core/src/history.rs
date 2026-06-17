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

/// SQL выборки транскрибаций одной записи (используется в `query` и
/// `get_transcriptions` — единый источник, чтобы колонки не разъезжались).
const TRANSCRIPTIONS_BY_RECORDING_SQL: &str = "SELECT id, recording_id, text, model_name, transcriber_type, created_at
     FROM transcriptions WHERE recording_id = ?1 ORDER BY created_at DESC";

/// Маппинг строки таблицы `recordings` → `Recording`. Чистый, разделяемый.
fn map_recording_row(row: &rusqlite::Row) -> rusqlite::Result<Recording> {
    Ok(Recording {
        id: row.get(0)?,
        audio_path: row.get(1)?,
        timestamp: row.get(2)?,
        duration_secs: row.get(3)?,
    })
}

/// Маппинг строки таблицы `transcriptions` → `Transcription`. Чистый, разделяемый.
fn map_transcription_row(row: &rusqlite::Row) -> rusqlite::Result<Transcription> {
    Ok(Transcription {
        id: row.get(0)?,
        recording_id: row.get(1)?,
        text: row.get(2)?,
        model_name: row.get(3)?,
        transcriber_type: row.get(4)?,
        created_at: row.get(5)?,
    })
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

        // DEBUG, а не INFO: открытие БД делается на каждом тике фоновых задач
        // (LRU-sweeper читает конфиг раз в минуту), иначе INFO-лог спамит
        // «БД истории открыта» в простое. Реальный сбой БД идёт по ветке
        // ошибки выше (ArcanaError::Database) и виден независимо от этого уровня.
        tracing::debug!("БД истории открыта: {:?}", db_path);
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
            .query_map(rusqlite::params![since_timestamp, limit, offset], map_recording_row)
            .map_err(|e| ArcanaError::Database(format!("Ошибка маппинга: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ArcanaError::Database(format!("Ошибка сбора: {}", e)))?;

        // Для каждой записи — загрузить транскрибации
        let mut entries = Vec::with_capacity(recordings.len());
        let mut trans_stmt = conn
            .prepare(TRANSCRIPTIONS_BY_RECORDING_SQL)
            .map_err(|e| ArcanaError::Database(format!("Ошибка запроса транскрибаций: {}", e)))?;

        for rec in recordings {
            let transcriptions: Vec<Transcription> = trans_stmt
                .query_map(rusqlite::params![rec.id], map_transcription_row)
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
            .prepare(TRANSCRIPTIONS_BY_RECORDING_SQL)
            .map_err(|e| ArcanaError::Database(format!("Ошибка запроса: {}", e)))?;

        let result = stmt
            .query_map(rusqlite::params![recording_id], map_transcription_row)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Создаёт временную БД истории в уникальной поддиректории temp.
    /// Имя должно быть уникальным per-test (тесты идут параллельно).
    fn temp_db(name: &str) -> (HistoryDB, PathBuf) {
        let dir = std::env::temp_dir().join(format!("arcanaglyph_test_history_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        let db = HistoryDB::new(&dir.join("history.db"), dir.join("audio")).expect("создание БД истории");
        (db, dir)
    }

    #[test]
    fn test_recording_and_transcription_roundtrip() {
        let (db, dir) = temp_db("roundtrip");
        let rec_id = db.add_recording("/tmp/a.raw", 5).unwrap();
        db.add_transcription(rec_id, "привет мир", "gigaam-v3", "gigaam")
            .unwrap();

        let (entries, total) = db.query(0, 10, 0).unwrap();
        assert_eq!(total, 1);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].recording.id, rec_id);
        assert_eq!(entries[0].recording.duration_secs, 5);
        assert_eq!(entries[0].transcriptions.len(), 1);
        assert_eq!(entries[0].transcriptions[0].text, "привет мир");

        let trans = db.get_transcriptions(rec_id).unwrap();
        assert_eq!(trans.len(), 1);
        assert_eq!(trans[0].model_name, "gigaam-v3");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_settings_get_set_replace() {
        let (db, dir) = temp_db("settings");
        assert_eq!(db.get_setting("missing"), None);
        db.set_setting("lang", "ru").unwrap();
        assert_eq!(db.get_setting("lang"), Some("ru".to_string()));
        // INSERT OR REPLACE — повторная запись по тому же ключу перезаписывает.
        db.set_setting("lang", "en").unwrap();
        assert_eq!(db.get_setting("lang"), Some("en".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_get_all_settings_returns_all_pairs() {
        let (db, dir) = temp_db("all_settings");
        assert!(db.get_all_settings().unwrap().is_empty());
        db.set_setting("lang", "ru").unwrap();
        db.set_setting("theme", "dark").unwrap();
        let all = db.get_all_settings().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all.get("lang").map(String::as_str), Some("ru"));
        assert_eq!(all.get("theme").map(String::as_str), Some("dark"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_audio_exists_checks_db_and_disk() {
        let (db, dir) = temp_db("audio_exists");
        // Несуществующая запись → false.
        assert!(!db.audio_exists(999));
        // Запись с путём к реальному файлу → true.
        let audio_path = dir.join("real.wav");
        std::fs::write(&audio_path, b"x").unwrap();
        let rec_real = db.add_recording(audio_path.to_str().unwrap(), 1).unwrap();
        assert!(db.audio_exists(rec_real));
        // Запись с путём к несуществующему файлу → false.
        let rec_missing = db.add_recording("/tmp/nope_xyz_arcanaglyph.wav", 1).unwrap();
        assert!(!db.audio_exists(rec_missing));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delete_recording_cascades_transcriptions() {
        let (db, dir) = temp_db("delete");
        let rec_id = db.add_recording("/tmp/b.raw", 3).unwrap();
        db.add_transcription(rec_id, "текст", "m", "t").unwrap();
        db.delete_recording(rec_id).unwrap();

        let (entries, total) = db.query(0, 10, 0).unwrap();
        assert_eq!(total, 0);
        assert!(entries.is_empty());
        // Каскад (ON DELETE CASCADE): транскрибации удалены вместе с записью.
        assert!(db.get_transcriptions(rec_id).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_clear_empties_history() {
        let (db, dir) = temp_db("clear");
        let r1 = db.add_recording("/tmp/c1.raw", 1).unwrap();
        db.add_transcription(r1, "t1", "m", "t").unwrap();
        db.add_recording("/tmp/c2.raw", 2).unwrap();
        db.clear().unwrap();
        let (_, total) = db.query(0, 10, 0).unwrap();
        assert_eq!(total, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cleanup_old_recordings_guard_and_recent() {
        let (db, dir) = temp_db("cleanup");
        db.add_recording("/tmp/recent.raw", 1).unwrap();
        // hours == 0 → выключено, ничего не удаляет.
        assert_eq!(db.cleanup_old_recordings(0).unwrap(), 0);
        // Свежая запись не старше 24ч → не удаляется.
        assert_eq!(db.cleanup_old_recordings(24).unwrap(), 0);
        let (_, total) = db.query(0, 10, 0).unwrap();
        assert_eq!(total, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_export_txt_and_csv() {
        let (db, dir) = temp_db("export");
        let rec_id = db.add_recording("/tmp/e.raw", 7).unwrap();
        db.add_transcription(rec_id, "образец текста", "gigaam-v3", "gigaam")
            .unwrap();

        let txt = db.export("txt").unwrap();
        assert!(txt.contains("образец текста"));
        assert!(txt.contains("gigaam-v3"));

        let csv = db.export("csv").unwrap();
        assert!(csv.starts_with('\u{FEFF}')); // BOM для Excel
        assert!(csv.contains("Дата;Длительность"));
        assert!(csv.contains("образец текста"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
