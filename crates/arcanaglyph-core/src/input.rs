// crates/arcanaglyph-core/src/input.rs

use crate::error::ArcanaError;
use std::process::Command;

/// Определяет, работаем ли мы на Wayland
fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
}

/// Вставляет текст туда, где стоит курсор.
/// На Wayland: копирует в clipboard через wl-copy, затем Ctrl+V через wtype.
/// На X11: использует enigo для прямой эмуляции ввода.
pub fn type_text(text: &str) -> Result<(), ArcanaError> {
    if text.is_empty() {
        return Ok(());
    }

    if is_wayland() {
        type_text_wayland(text)
    } else {
        type_text_x11(text)
    }
}

/// Вставка через clipboard на Wayland: wl-copy → wtype Ctrl+v
fn type_text_wayland(text: &str) -> Result<(), ArcanaError> {
    // Копируем текст в clipboard
    let mut child = Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось запустить wl-copy: {} (установите: sudo apt install wl-clipboard)", e)))?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось передать текст в wl-copy: {}", e)))?;
    }

    child
        .wait()
        .map_err(|e| ArcanaError::InputSimulation(format!("wl-copy завершился с ошибкой: {}", e)))?;

    // Небольшая пауза, чтобы clipboard успел обновиться
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Симулируем Ctrl+V через wtype
    let status = Command::new("wtype")
        .args(["-M", "ctrl", "v", "-m", "ctrl"])
        .status()
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось запустить wtype: {} (установите: sudo apt install wtype)", e)))?;

    if !status.success() {
        return Err(ArcanaError::InputSimulation("wtype завершился с ошибкой".into()));
    }

    tracing::info!("Текст вставлен через clipboard ({} символов)", text.len());
    Ok(())
}

/// Вставка через enigo на X11
fn type_text_x11(text: &str) -> Result<(), ArcanaError> {
    use enigo::{Enigo, Keyboard, Settings};

    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось создать Enigo: {}", e)))?;

    enigo
        .text(text)
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось вставить текст: {}", e)))?;

    tracing::info!("Текст вставлен в активное окно ({} символов)", text.len());
    Ok(())
}
