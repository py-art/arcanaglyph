// crates/arcanaglyph-core/src/db.rs
//
// Система миграций БД.
// SQL файлы хранятся в migrations/ в корне проекта.
// При добавлении новой миграции:
// 1. Создать файл migrations/vNNN_description.sql
// 2. Добавить include_str!() в MIGRATIONS
// 3. Увеличить SCHEMA_VERSION

use crate::error::ArcanaError;
use rusqlite::Connection;

/// SQL миграции, встроенные при компиляции
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "v001_initial_schema",
        include_str!("../../../migrations/v001_initial_schema.sql"),
    ),
    (
        "v002_settings_table",
        include_str!("../../../migrations/v002_settings_table.sql"),
    ),
];

/// Текущая версия схемы = количество миграций
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// Получить версию схемы из БД
pub fn get_version(conn: &Connection) -> u32 {
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if !exists {
        return 0;
    }

    conn.query_row("SELECT version FROM schema_version LIMIT 1", [], |row| row.get(0))
        .unwrap_or(0)
}

/// Установить версию схемы
fn set_version(conn: &Connection, version: u32) -> Result<(), ArcanaError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")
        .map_err(|e| ArcanaError::Database(format!("schema_version: {}", e)))?;
    conn.execute("DELETE FROM schema_version", [])
        .map_err(|e| ArcanaError::Database(format!("schema_version clear: {}", e)))?;
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        rusqlite::params![version],
    )
    .map_err(|e| ArcanaError::Database(format!("schema_version set: {}", e)))?;
    Ok(())
}

/// Применить все pending миграции
pub fn run_migrations(conn: &Connection) -> Result<(), ArcanaError> {
    let current = get_version(conn);

    if current as usize == MIGRATIONS.len() {
        return Ok(());
    }

    if current as usize > MIGRATIONS.len() {
        return Err(ArcanaError::Database(format!(
            "БД версии {} новее приложения ({}). Обновите приложение.",
            current, SCHEMA_VERSION
        )));
    }

    tracing::info!("Миграция БД: v{} → v{}", current, SCHEMA_VERSION);

    for (i, (name, sql)) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as u32;
        if version <= current {
            continue;
        }

        // Для v1: проверяем совместимость со старой БД без миграций
        if version == 1 {
            let has_tables: bool = conn
                .query_row(
                    "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='recordings'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(false);

            if has_tables {
                tracing::info!("Обнаружена БД от предыдущей версии, совместима");
                set_version(conn, version)?;
                continue;
            }
        }

        conn.execute_batch(sql)
            .map_err(|e| ArcanaError::Database(format!("Миграция {}: {}", name, e)))?;
        set_version(conn, version)?;
        tracing::info!("Миграция {}: применена (v{})", name, version);
    }

    tracing::info!("Миграция БД завершена: v{}", SCHEMA_VERSION);
    Ok(())
}
