// crates/arcanaglyph-core/src/input.rs

use crate::error::ArcanaError;
use enigo::{Enigo, Keyboard, Settings};

/// Вставляет текст в активное окно, эмулируя ввод с клавиатуры.
/// Использует enigo для кросс-платформенной симуляции ввода.
pub fn type_text(text: &str) -> Result<(), ArcanaError> {
    if text.is_empty() {
        return Ok(());
    }

    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось создать Enigo: {}", e)))?;

    enigo
        .text(text)
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось вставить текст: {}", e)))?;

    tracing::info!("Текст вставлен в активное окно ({} символов)", text.len());
    Ok(())
}
