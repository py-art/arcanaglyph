// crates/arcanaglyph-app/src/models/mod.rs
//
// Скачивание моделей, распаковка, валидация и Tauri-команды реестра/удаления/
// инсталляции. `is_model_installed` живёт в `registry` и реэкспортируется как
// `pub(crate)` для использования в `download::ensure_active_model`.

pub mod download;
pub mod registry;
