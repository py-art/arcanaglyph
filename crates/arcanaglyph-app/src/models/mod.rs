// crates/arcanaglyph-app/src/models/mod.rs
//
// Скачивание моделей, распаковка, валидация и Tauri-команды реестра/удаления/
// инсталляции. `is_model_installed` живёт в нейтральном `installed`, чтобы и
// `registry`, и `download::ensure_active_model` зависели от него без цикла
// `download ↔ registry`.

pub mod download;
pub mod installed;
pub mod registry;
