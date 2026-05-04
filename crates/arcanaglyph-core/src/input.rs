// crates/arcanaglyph-core/src/input.rs
//
// Вставка распознанного текста в активное окно.
//
// На Linux: ветвление по сессии.
//   Wayland — wl-copy + XDG RemoteDesktop portal (Shift+Insert).
//   X11    — arboard (clipboard) + enigo (Shift+Insert).
// На Windows/macOS: enigo.text() напрямую (быстрый нативный backend).
//
// Почему на X11 не enigo.text(): enigo на X11 для каждого не-ASCII символа
// делает XKB keysym remap (XChangeKeyboardMapping → XkbMapNotify по всем клиентам).
// На слабом CPU без AVX (Intel Celeron N5095) это даёт задержки 20-40с на 75 символов,
// фризит сессию и периодически портит часть символов из-за гонок раскладки.
// Clipboard + Shift+Insert использует только 2 стабильных keysym'а (Shift_L, Insert),
// которые есть в любой раскладке — никакого ремаппинга, вставка мгновенная.
//
// Linux-only зависимости (`ashpd`, `wl-copy`, `arboard`, evdev keycodes) изолированы
// за `cfg`, чтобы крейт компилировался под Windows/macOS без изменений.

use crate::error::ArcanaError;

/// Вставляет текст туда, где стоит курсор.
pub async fn type_text(text: &str) -> Result<(), ArcanaError> {
    if text.is_empty() {
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        if is_wayland() {
            type_text_wayland(text).await
        } else {
            type_text_x11(text).await
        }
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        type_text_enigo(text)
    }
}

/// Вставка через enigo.text() — посимвольная симуляция нажатий.
/// Используется только на Windows/macOS, где нативный backend быстрый.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn type_text_enigo(text: &str) -> Result<(), ArcanaError> {
    use enigo::{Enigo, Keyboard, Settings};

    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось создать Enigo: {}", e)))?;

    enigo
        .text(text)
        .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось вставить текст: {}", e)))?;

    tracing::info!("Текст вставлен в активное окно ({} символов)", text.len());
    Ok(())
}

// =====================================================================
// Linux-only: общие импорты для Wayland/X11 путей
// =====================================================================

#[cfg(target_os = "linux")]
use std::process::Command;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
#[cfg(target_os = "linux")]
use tokio::sync::Mutex;

/// Определяет, работаем ли мы на Wayland
#[cfg(target_os = "linux")]
fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
}

// =====================================================================
// Linux X11: arboard (clipboard) + enigo (Shift+Insert)
// =====================================================================

/// Глобальный Clipboard arboard. arboard на X11 держит фоновый поток,
/// который отвечает на SelectionRequest от целевого приложения. Если уронить
/// Clipboard, поток умрёт и приложение не сможет прочитать данные. Поэтому
/// держим инстанс на всё время жизни процесса.
#[cfg(target_os = "linux")]
static X11_CLIPBOARD: OnceLock<std::sync::Mutex<Option<arboard::Clipboard>>> = OnceLock::new();

#[cfg(target_os = "linux")]
async fn type_text_x11(text: &str) -> Result<(), ArcanaError> {
    let text_owned = text.to_string();
    let len = text_owned.len();

    // arboard блокирующий — выполняем в spawn_blocking.
    tokio::task::spawn_blocking(move || -> Result<(), ArcanaError> {
        use arboard::{LinuxClipboardKind, SetExtLinux};

        let mutex = X11_CLIPBOARD.get_or_init(|| std::sync::Mutex::new(None));
        let mut guard = mutex
            .lock()
            .map_err(|e| ArcanaError::InputSimulation(format!("X11 clipboard mutex poisoned: {}", e)))?;

        if guard.is_none() {
            *guard =
                Some(arboard::Clipboard::new().map_err(|e| {
                    ArcanaError::InputSimulation(format!("Не удалось инициализировать clipboard: {}", e))
                })?);
        }
        let cb = guard.as_mut().expect("clipboard initialized above");

        // CLIPBOARD selection — Ctrl+V и Shift+Insert в GTK/Qt/WebKit GUI.
        cb.set_text(text_owned.clone())
            .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось скопировать в clipboard: {}", e)))?;
        // PRIMARY selection — Shift+Insert в xterm/urxvt/некоторых терминалах
        // вставляет именно из PRIMARY, не из CLIPBOARD.
        cb.set()
            .clipboard(LinuxClipboardKind::Primary)
            .text(text_owned)
            .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось скопировать в primary: {}", e)))?;
        Ok(())
    })
    .await
    .map_err(|e| ArcanaError::InputSimulation(format!("spawn_blocking (clipboard): {}", e)))??;

    // Небольшая пауза, чтобы X-сервер успел зарегистрировать смену владельца selection.
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    // Shift+Insert через enigo. В отличие от .text() здесь только 2 стабильных
    // keysym'а — никакого XKB-ремаппинга, выполняется мгновенно даже на N5095.
    tokio::task::spawn_blocking(|| -> Result<(), ArcanaError> {
        use enigo::{Direction, Enigo, Key, Keyboard, Settings};

        let mut enigo = Enigo::new(&Settings::default())
            .map_err(|e| ArcanaError::InputSimulation(format!("Не удалось создать Enigo: {}", e)))?;
        enigo
            .key(Key::Shift, Direction::Press)
            .map_err(|e| ArcanaError::InputSimulation(format!("Ошибка нажатия Shift: {}", e)))?;
        enigo
            .key(Key::Insert, Direction::Click)
            .map_err(|e| ArcanaError::InputSimulation(format!("Ошибка Insert: {}", e)))?;
        enigo
            .key(Key::Shift, Direction::Release)
            .map_err(|e| ArcanaError::InputSimulation(format!("Ошибка отпускания Shift: {}", e)))?;
        Ok(())
    })
    .await
    .map_err(|e| ArcanaError::InputSimulation(format!("spawn_blocking (paste): {}", e)))??;

    tracing::info!(
        "Текст вставлен в активное окно через clipboard + Shift+Insert ({} символов)",
        len
    );
    Ok(())
}

// =====================================================================
// Linux Wayland: XDG RemoteDesktop portal + wl-copy
// =====================================================================

/// Состояние RemoteDesktop сессии (переиспользуется между вызовами)
#[cfg(target_os = "linux")]
struct RdSession {
    proxy: ashpd::desktop::remote_desktop::RemoteDesktop,
    session: ashpd::desktop::Session<ashpd::desktop::remote_desktop::RemoteDesktop>,
}

/// Глобальная сессия RemoteDesktop (создаётся один раз)
#[cfg(target_os = "linux")]
static RD_SESSION: OnceLock<Mutex<Option<RdSession>>> = OnceLock::new();

/// Путь к файлу с restore_token для сохранения разрешения между запусками
#[cfg(target_os = "linux")]
fn restore_token_path() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("com", "arcanaglyph", "ArcanaGlyph")
        .map(|dirs| dirs.data_dir().join("rd_restore_token"))
}

/// Загружает restore_token из файла
#[cfg(target_os = "linux")]
fn load_restore_token() -> Option<String> {
    let path = restore_token_path()?;
    std::fs::read_to_string(&path).ok().filter(|s| !s.is_empty())
}

/// Сохраняет restore_token в файл
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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
