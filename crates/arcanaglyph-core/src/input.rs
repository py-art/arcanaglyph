// crates/arcanaglyph-core/src/input.rs

use crate::error::ArcanaError;
use std::process::Command;
use std::sync::OnceLock;
use tokio::sync::Mutex;

/// Определяет, работаем ли мы на Wayland
fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
}

/// Вставляет текст туда, где стоит курсор.
/// На Wayland: копирует в clipboard через wl-copy, затем Ctrl+V через XDG RemoteDesktop portal.
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

/// Состояние RemoteDesktop сессии (переиспользуется между вызовами)
struct RdSession {
    proxy: ashpd::desktop::remote_desktop::RemoteDesktop,
    session: ashpd::desktop::Session<ashpd::desktop::remote_desktop::RemoteDesktop>,
}

/// Глобальная сессия RemoteDesktop (создаётся один раз)
static RD_SESSION: OnceLock<Mutex<Option<RdSession>>> = OnceLock::new();

/// Инициализирует RemoteDesktop сессию (при первом вызове покажется диалог GNOME)
async fn init_rd_session() -> Result<RdSession, ArcanaError> {
    use ashpd::desktop::remote_desktop::{DeviceType, RemoteDesktop, SelectDevicesOptions};
    use ashpd::enumflags2::BitFlags;

    tracing::info!("Инициализация XDG RemoteDesktop сессии...");

    let proxy = RemoteDesktop::new()
        .await
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось подключиться к RemoteDesktop порталу: {}", e)))?;

    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось создать RemoteDesktop сессию: {}", e)))?;

    proxy
        .select_devices(
            &session,
            SelectDevicesOptions::default().set_devices(BitFlags::from(DeviceType::Keyboard)),
        )
        .await
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось выбрать устройства: {}", e)))?;

    proxy
        .start(&session, None, Default::default())
        .await
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось запустить RemoteDesktop сессию: {}", e)))?
        .response()
        .map_err(|e| ArcanaError::InputSimulation(format!("Пользователь отклонил запрос RemoteDesktop: {}", e)))?;

    tracing::info!("RemoteDesktop сессия успешно создана");
    Ok(RdSession { proxy, session })
}

/// Симулирует Ctrl+V через XDG RemoteDesktop portal
async fn simulate_ctrl_v(rd: &RdSession) -> Result<(), ArcanaError> {
    use ashpd::desktop::remote_desktop::KeyState;

    // KEY_LEFTCTRL = 29, KEY_V = 47 (Linux evdev keycodes)
    let opts = Default::default();

    rd.proxy.notify_keyboard_keycode(&rd.session, 29, KeyState::Pressed, opts).await
        .map_err(|e| ArcanaError::InputSimulation(format!("Ошибка нажатия Ctrl: {}", e)))?;

    rd.proxy.notify_keyboard_keycode(&rd.session, 47, KeyState::Pressed, Default::default()).await
        .map_err(|e| ArcanaError::InputSimulation(format!("Ошибка нажатия V: {}", e)))?;

    rd.proxy.notify_keyboard_keycode(&rd.session, 47, KeyState::Released, Default::default()).await
        .map_err(|e| ArcanaError::InputSimulation(format!("Ошибка отпускания V: {}", e)))?;

    rd.proxy.notify_keyboard_keycode(&rd.session, 29, KeyState::Released, Default::default()).await
        .map_err(|e| ArcanaError::InputSimulation(format!("Ошибка отпускания Ctrl: {}", e)))?;

    Ok(())
}

/// Копирует текст в Wayland clipboard через wl-copy
fn copy_to_clipboard(text: &str) -> Result<(), ArcanaError> {
    let mut child = Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| ArcanaError::InputSimulation(format!(
            "Не удалось запустить wl-copy: {} (установите: sudo apt install wl-clipboard)", e
        )))?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin.write_all(text.as_bytes())
            .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось передать текст в wl-copy: {}", e)))?;
    }

    child.wait()
        .map_err(|e| ArcanaError::InputSimulation(format!("wl-copy завершился с ошибкой: {}", e)))?;

    Ok(())
}

/// Вставка через clipboard на Wayland: wl-copy → XDG RemoteDesktop Ctrl+V
fn type_text_wayland(text: &str) -> Result<(), ArcanaError> {
    // Копируем текст в clipboard
    copy_to_clipboard(text)?;

    // Небольшая пауза, чтобы clipboard успел обновиться
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Симулируем Ctrl+V через RemoteDesktop portal
    let handle = tokio::runtime::Handle::current();
    handle.block_on(async {
        let mutex = RD_SESSION.get_or_init(|| Mutex::new(None));
        let mut guard = mutex.lock().await;

        // Если сессии нет или она сломалась — создаём новую
        if guard.is_none() {
            match init_rd_session().await {
                Ok(session) => *guard = Some(session),
                Err(e) => {
                    tracing::warn!("RemoteDesktop недоступен: {}. Текст скопирован в буфер — нажмите Ctrl+V.", e);
                    return Ok(());
                }
            }
        }

        if let Some(rd) = guard.as_ref() {
            if let Err(e) = simulate_ctrl_v(rd).await {
                tracing::warn!("Ошибка RemoteDesktop: {}. Пересоздаю сессию...", e);
                // Сессия сломалась — сбрасываем для пересоздания при следующем вызове
                *guard = None;
                // Текст уже в clipboard — пользователь может вставить сам
                tracing::info!("Текст скопирован в буфер обмена — нажмите Ctrl+V для вставки");
            } else {
                tracing::info!("Текст вставлен через RemoteDesktop ({} символов)", text.len());
            }
        }

        Ok(())
    })
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
