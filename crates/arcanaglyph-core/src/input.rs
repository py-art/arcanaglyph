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
pub async fn type_text(text: &str) -> Result<(), ArcanaError> {
    if text.is_empty() {
        return Ok(());
    }

    if is_wayland() {
        type_text_wayland(text).await
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

/// Путь к файлу с restore_token для сохранения разрешения между запусками
fn restore_token_path() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("com", "arcanaglyph", "ArcanaGlyph")
        .map(|dirs| dirs.data_dir().join("rd_restore_token"))
}

/// Загружает restore_token из файла
fn load_restore_token() -> Option<String> {
    let path = restore_token_path()?;
    std::fs::read_to_string(&path).ok().filter(|s| !s.is_empty())
}

/// Сохраняет restore_token в файл
fn save_restore_token(token: &str) {
    if let Some(path) = restore_token_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, token);
    }
}

/// Инициализирует RemoteDesktop сессию.
/// При первом вызове GNOME покажет диалог подтверждения.
/// При последующих запусках используется сохранённый restore_token.
async fn init_rd_session() -> Result<RdSession, ArcanaError> {
    use ashpd::desktop::PersistMode;
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

    // Загружаем сохранённый токен для восстановления без диалога
    let restore_token = load_restore_token();
    let mut opts = SelectDevicesOptions::default()
        .set_devices(BitFlags::from(DeviceType::Keyboard))
        .set_persist_mode(PersistMode::ExplicitlyRevoked);
    if let Some(ref token) = restore_token {
        tracing::info!("Восстанавливаю RemoteDesktop сессию из сохранённого токена");
        opts = opts.set_restore_token(token.as_str());
    }

    proxy
        .select_devices(&session, opts)
        .await
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось выбрать устройства: {}", e)))?;

    let response = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось запустить RemoteDesktop сессию: {}", e)))?
        .response()
        .map_err(|e| ArcanaError::InputSimulation(format!("Пользователь отклонил запрос RemoteDesktop: {}", e)))?;

    // Сохраняем новый restore_token для будущих запусков
    if let Some(token) = response.restore_token() {
        save_restore_token(token);
        tracing::info!("RemoteDesktop restore_token сохранён");
    }

    tracing::info!("RemoteDesktop сессия успешно создана");
    Ok(RdSession { proxy, session })
}

/// Симулирует Shift+Insert через XDG RemoteDesktop portal.
/// Shift+Insert — универсальная вставка из clipboard на Linux,
/// работает и в терминалах, и в GUI-приложениях (GTK, Qt, браузеры).
async fn simulate_paste(rd: &RdSession) -> Result<(), ArcanaError> {
    use ashpd::desktop::remote_desktop::KeyState;

    // KEY_LEFTSHIFT = 42, KEY_INSERT = 110 (Linux evdev keycodes)
    const SHIFT: i32 = 42;
    const INSERT: i32 = 110;

    rd.proxy
        .notify_keyboard_keycode(&rd.session, SHIFT, KeyState::Pressed, Default::default())
        .await
        .map_err(|e| ArcanaError::InputSimulation(format!("Ошибка нажатия Shift: {}", e)))?;
    rd.proxy
        .notify_keyboard_keycode(&rd.session, INSERT, KeyState::Pressed, Default::default())
        .await
        .map_err(|e| ArcanaError::InputSimulation(format!("Ошибка нажатия Insert: {}", e)))?;
    rd.proxy
        .notify_keyboard_keycode(&rd.session, INSERT, KeyState::Released, Default::default())
        .await
        .map_err(|e| ArcanaError::InputSimulation(format!("Ошибка отпускания Insert: {}", e)))?;
    rd.proxy
        .notify_keyboard_keycode(&rd.session, SHIFT, KeyState::Released, Default::default())
        .await
        .map_err(|e| ArcanaError::InputSimulation(format!("Ошибка отпускания Shift: {}", e)))?;

    Ok(())
}

/// Копирует текст в Wayland clipboard (CLIPBOARD + PRIMARY selection) через wl-copy.
/// PRIMARY нужен потому что Shift+Insert в некоторых терминалах вставляет из PRIMARY.
fn copy_to_clipboard(text: &str) -> Result<(), ArcanaError> {
    use std::io::Write;

    // CLIPBOARD (основной буфер)
    let mut child = Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            ArcanaError::InputSimulation(format!(
                "Не удалось запустить wl-copy: {} (установите: sudo apt install wl-clipboard)",
                e
            ))
        })?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось передать текст в wl-copy: {}", e)))?;
    }
    child
        .wait()
        .map_err(|e| ArcanaError::InputSimulation(format!("wl-copy завершился с ошибкой: {}", e)))?;

    // PRIMARY selection (буфер выделения, используется Shift+Insert в некоторых терминалах)
    let mut child_primary = Command::new("wl-copy")
        .arg("--primary")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось запустить wl-copy --primary: {}", e)))?;
    if let Some(stdin) = child_primary.stdin.as_mut() {
        stdin.write_all(text.as_bytes()).map_err(|e| {
            ArcanaError::InputSimulation(format!("Не удалось передать текст в wl-copy --primary: {}", e))
        })?;
    }
    child_primary
        .wait()
        .map_err(|e| ArcanaError::InputSimulation(format!("wl-copy --primary завершился с ошибкой: {}", e)))?;

    Ok(())
}

/// Вставка через clipboard на Wayland: wl-copy → XDG RemoteDesktop Ctrl+V
async fn type_text_wayland(text: &str) -> Result<(), ArcanaError> {
    // Копируем текст в clipboard
    copy_to_clipboard(text)?;

    // Небольшая пауза, чтобы clipboard успел обновиться
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Симулируем Ctrl+V через RemoteDesktop portal
    let mutex = RD_SESSION.get_or_init(|| Mutex::new(None));
    let mut guard = mutex.lock().await;

    // Если сессии нет — создаём новую
    if guard.is_none() {
        match init_rd_session().await {
            Ok(session) => *guard = Some(session),
            Err(e) => {
                tracing::warn!(
                    "RemoteDesktop недоступен: {}. Текст скопирован в буфер — нажмите Ctrl+V.",
                    e
                );
                return Ok(());
            }
        }
    }

    if let Some(rd) = guard.as_ref() {
        if let Err(e) = simulate_paste(rd).await {
            tracing::warn!("Ошибка RemoteDesktop: {}. Пересоздаю сессию...", e);
            *guard = None;
            tracing::info!("Текст скопирован в буфер обмена — нажмите Ctrl+V для вставки");
        } else {
            tracing::info!("Текст вставлен через RemoteDesktop ({} символов)", text.len());
        }
    }

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
