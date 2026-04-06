-- v002: Таблица настроек приложения
-- Key-value хранилище конфигурации (заменяет config.toml)

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) WITHOUT ROWID;
