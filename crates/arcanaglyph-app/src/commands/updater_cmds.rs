// crates/arcanaglyph-app/src/commands/updater_cmds.rs
//
// Команды UI-баннера обновлений: ручная проверка, dismiss, открытие release
// notes, запуск установки `install.sh` в внешнем терминале, restart_app.
// Реальное обновление state крутится в `crate::updater` — здесь только Tauri-обёртки.

use crate::updater;
use arcanaglyph_core::history::HistoryDB;
use std::sync::Arc;

/// Tauri-команда: ручная проверка обновлений (кнопка в About). Делает
/// HTTP-запрос к GitHub API, обновляет state, возвращает UpdateInfo
/// если есть newer (and not dismissed).
#[tauri::command]
pub async fn check_updates_now(
    history_db: tauri::State<'_, Arc<HistoryDB>>,
) -> Result<Option<updater::UpdateInfo>, String> {
    let db = history_db.inner().clone();
    updater::check_for_update(&db).await.map_err(|e| e.to_string())
}

/// Tauri-команда: записать `version` в `dismissed_version`. Баннер
/// для этой версии больше не появится, пока не выйдет ещё более новая.
#[tauri::command]
pub fn dismiss_update(version: String, history_db: tauri::State<'_, Arc<HistoryDB>>) -> Result<(), String> {
    updater::dismiss(history_db.inner(), &version).map_err(|e| e.to_string())
}

/// Tauri-команда: открыть URL в браузере. Используется кнопкой
/// «Что нового» — ведёт на release page.
#[tauri::command]
pub fn open_release_notes(url: String) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(&url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("xdg-open: {e}"))
}

/// Возвращает первый найденный terminal-emulator из known list.
/// Использует `<term> --version` спавн-проверку (без `which` crate).
fn detect_terminal() -> Option<&'static str> {
    const TERMINALS: &[&str] = &["gnome-terminal", "konsole", "kitty", "alacritty", "tilix", "xterm"];
    for &t in TERMINALS {
        let ok = std::process::Command::new(t)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Some(t);
        }
    }
    None
}

/// Per-terminal CLI-аргументы для запуска `bash -c <cmd>`.
fn terminal_args(terminal: &str, bash_cmd: &str) -> Vec<String> {
    let cmd = bash_cmd.to_string();
    match terminal {
        // gnome-terminal/tilix принимают подкоманду после `--`.
        "gnome-terminal" | "tilix" => {
            vec!["--".into(), "bash".into(), "-c".into(), cmd]
        }
        // konsole закрывает окно сразу после exit команды без --noclose.
        "konsole" => vec!["--noclose".into(), "-e".into(), "bash".into(), "-c".into(), cmd],
        // kitty / alacritty / xterm: -e и далее команда + args.
        _ => vec!["-e".into(), "bash".into(), "-c".into(), cmd],
    }
}

/// Tauri-команда: запустить установку новой версии. Скачиваем install.sh
/// в temp-файл, спавним терминал с `bash <tmp>` (не `curl|bash` — это
/// устраняет PTY-passthrough проблемы для sudo внутри install.sh).
/// При отсутствии терминала возвращаем ошибку с инструкцией для UI.
///
/// `latest_version` пишется в `UpdateState.applying_version` ДО спавна
/// терминала: UI на этом основании переключается в applying-режим и
/// сохраняет его между перезапусками (пока `APP_VERSION` не догонит).
#[tauri::command]
pub async fn apply_update(latest_version: String, history_db: tauri::State<'_, Arc<HistoryDB>>) -> Result<(), String> {
    // Помечаем applying ДО любых сетевых/IO операций — UI получает
    // мгновенный переход в applying-режим и persistent state на случай
    // повторного запуска приложения до restart.
    updater::set_applying(history_db.inner(), &latest_version).map_err(|e| e.to_string())?;

    // Если ниже что-то падает — откатываем applying, иначе баннер
    // залипнет в applying-режиме без реально начавшейся установки.
    let result = apply_update_inner().await;
    if result.is_err() {
        let _ = updater::clear_applying(history_db.inner());
    }
    result
}

async fn apply_update_inner() -> Result<(), String> {
    let terminal = detect_terminal().ok_or_else(|| {
        format!(
            "Терминал не найден. Запустите вручную:\n  curl -fsSL {} | bash",
            updater::INSTALL_URL
        )
    })?;

    // Скачиваем install.sh.
    let body = reqwest::Client::builder()
        .user_agent(format!("arcanaglyph/{}", updater::APP_VERSION))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?
        .get(updater::INSTALL_URL)
        .send()
        .await
        .map_err(|e| format!("Скачивание install.sh: {e}"))?
        .error_for_status()
        .map_err(|e| format!("install.sh HTTP: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Чтение install.sh: {e}"))?;

    // Сохраняем во временный файл (.sh suffix для удобства, не функционально).
    let tmp = tempfile::Builder::new()
        .prefix("arcanaglyph-update-")
        .suffix(".sh")
        .tempfile()
        .map_err(|e| format!("tempfile: {e}"))?;
    use std::io::Write;
    {
        let mut handle = tmp.as_file();
        handle
            .write_all(body.as_bytes())
            .map_err(|e| format!("Запись install.sh: {e}"))?;
    }
    // .keep() оставляет файл на диске после drop — терминал должен успеть
    // его прочитать. Файл лежит в /tmp/, очистится при reboot.
    let path = tmp.into_temp_path().keep().map_err(|e| format!("temp keep: {e}"))?;

    // Wrapper держит окно открытым после exit, чтобы пользователь увидел
    // сообщение об ошибке если apt/sudo не дошёл до конца.
    let bash_cmd = format!(
        "bash {} ; ec=$?; echo; echo \"Exit: $ec\"; echo 'Press Enter to close'; read",
        path.display()
    );
    let args = terminal_args(terminal, &bash_cmd);

    std::process::Command::new(terminal)
        .args(&args)
        .spawn()
        .map_err(|e| format!("Запуск {terminal}: {e}"))?;

    Ok(())
}

/// Tauri-команда: сбросить applying-метку (пользователь нажал × в
/// applying-баннере). Не отменяет уже запущенную установку — только
/// прячет UI; при следующем check available-баннер вернётся если
/// версия всё ещё > APP_VERSION и не dismissed.
#[tauri::command]
pub fn clear_update_applying(history_db: tauri::State<'_, Arc<HistoryDB>>) -> Result<(), String> {
    updater::clear_applying(history_db.inner()).map_err(|e| e.to_string())
}

/// Tauri-команда: текущее значение `applying_version` из state.
/// Фронт зовёт на mount баннера, чтобы восстановить applying-режим
/// без ожидания emit'а из setup hook.
#[tauri::command]
pub fn get_update_applying(history_db: tauri::State<'_, Arc<HistoryDB>>) -> Option<String> {
    updater::applying_version(history_db.inner())
}

/// Tauri-команда: перезапустить приложение. Спавним новый процесс
/// (новая версия из PATH после успешной установки .deb) и выходим
/// из текущего. Если бинарь не найден в PATH — возвращаем Err, фронт
/// показывает toast.
#[tauri::command]
pub fn restart_app(app: tauri::AppHandle) -> Result<(), String> {
    // Detached spawn — новый процесс не зависит от родителя, который
    // через миллисекунды exit'нется.
    std::process::Command::new("arcanaglyph")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Запуск новой версии: {e}"))?;

    // Небольшая задержка чтобы spawn успел запуститься до exit
    // (на быстрых машинах race не случается, но safer side).
    std::thread::sleep(std::time::Duration::from_millis(150));
    app.exit(0);
    Ok(())
}
