// crates/arcanaglyph-app/src/commands/mod.rs
//
// Tauri-команды, сгруппированные по доменам. Каждый под-модуль реэкспортирует
// свои `#[tauri::command]` функции, чтобы `tauri::generate_handler!` в main.rs
// мог сослаться на них по короткому пути `commands::trigger` и т.д.

use arcanaglyph_core::ArcanaEngine;
use std::sync::{Arc, OnceLock};

/// Тип state для engine — инициализируется в фоне после показа окна.
pub type EngineState = Arc<OnceLock<ArcanaEngine>>;

/// Получить engine или вернуть ошибку «модель загружается».
pub(crate) fn get_engine(state: &EngineState) -> Result<&ArcanaEngine, String> {
    state.get().ok_or_else(|| "Модель загружается...".to_string())
}

pub mod config_history;
pub mod engine;
pub mod hotkeys;
pub mod platform;
pub mod updater_cmds;
pub mod widget_ext;

pub use config_history::*;
pub use engine::*;
pub use hotkeys::*;
pub use platform::*;
pub use updater_cmds::*;
pub use widget_ext::*;
