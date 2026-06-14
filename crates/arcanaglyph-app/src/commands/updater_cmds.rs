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

/// Открывает URL в системном браузере. Кроссплатформенно: `xdg-open` на Linux,
/// `cmd /C start` на Windows (пустой "" — обязательный аргумент-заголовок для
/// `start`), `open` на macOS.
fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let (program, args): (&str, Vec<&str>) = ("cmd", vec!["/C", "start", "", url]);
    #[cfg(target_os = "macos")]
    let (program, args): (&str, Vec<&str>) = ("open", vec![url]);
    #[cfg(target_os = "linux")]
    let (program, args): (&str, Vec<&str>) = ("xdg-open", vec![url]);

    std::process::Command::new(program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{program}: {e}"))
}

/// Tauri-команда: открыть URL в браузере. Используется кнопкой
/// «Что нового» — ведёт на release page.
#[tauri::command]
pub fn open_release_notes(url: String) -> Result<(), String> {
    open_url(&url)
}

/// Возвращает первый найденный terminal-emulator из known list.
/// Использует `<term> --version` спавн-проверку (без `which` crate).
/// Linux-only: на Windows/macOS in-app установки через терминал нет.
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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
pub async fn apply_update(
    latest_version: String,
    app: tauri::AppHandle,
    history_db: tauri::State<'_, Arc<HistoryDB>>,
) -> Result<(), String> {
    // Windows: авто-скачивание .exe-установщика и запуск, затем выход из
    // приложения — NSIS (currentUser, без админ-прав) заменит файлы запущенного
    // процесса. applying-метку выставляем как на Linux: переживёт в SQLite и
    // очистится после апдейта, когда APP_VERSION догонит её.
    #[cfg(target_os = "windows")]
    {
        updater::set_applying(history_db.inner(), &latest_version).map_err(|e| e.to_string())?;
        return match apply_update_windows().await {
            Ok(()) => {
                // Установщик запущен (независимый процесс — переживёт exit).
                // Выходим, чтобы разблокировать файлы для установки.
                app.exit(0);
                Ok(())
            }
            Err(e) => {
                // Не дошли до запуска — снимаем applying, иначе баннер залипнет.
                let _ = updater::clear_applying(history_db.inner());
                Err(e)
            }
        };
    }

    // macOS: in-app установки пока нет — открываем страницу релиза в браузере,
    // пользователь скачивает и ставит вручную. applying НЕ выставляем (нет
    // процесса установки, который его снимет — иначе баннер залипнет).
    #[cfg(target_os = "macos")]
    {
        let _ = (&history_db, &app);
        let url = format!("https://github.com/py-art/arcanaglyph/releases/tag/v{latest_version}");
        return open_url(&url);
    }

    // Linux: запускаем install.sh во внешнем терминале.
    #[cfg(target_os = "linux")]
    {
        let _ = &app;
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
}

#[cfg(target_os = "linux")]
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

    // Флаг успешной установки: терминальная обёртка создаёт его при exit 0
    // install.sh. UI ждёт этот флаг перед разблокировкой «Перезапустить»
    // (баг #3 — иначе можно перезапустить СТАРУЮ версию до конца установки).
    // Снимаем stale-флаг прошлой попытки перед стартом.
    let ready_flag = update_ready_flag_path();
    let _ = std::fs::remove_file(&ready_flag);

    // Преамбула: до запуска install.sh печатаем пошаговую инструкцию. Без неё
    // окно терминала на этапе «100% / пустой курсор» выглядит как «зависло» —
    // даже автор приложения не понимал, что делать (см. UPDATER-UX-BUGS.md).
    // Wrapper держит окно открытым после exit (echo 'Press Enter to close';
    // read), чтобы пользователь увидел сообщение об ошибке если apt/sudo не
    // дошёл до конца. При exit 0 создаём ready-флаг — сигнал UI, что установка
    // реально завершилась и «Перезапустить» можно разблокировать.
    let bash_cmd = format!(
        "echo '=== Обновление ArcanaGlyph ==='; \
         echo 'Сейчас будет скачан и установлен новый пакет.'; \
         echo '1. Дождитесь окончания скачивания (проценты дойдут до 100%).'; \
         echo '2. Затем потребуется ВВОД ПАРОЛЯ (sudo) — введите пароль пользователя'; \
         echo '   и нажмите Enter (символы при вводе не отображаются).'; \
         echo '3. После установки появится строка «Press Enter to close» — нажмите Enter.'; \
         echo '4. Вернитесь в приложение и нажмите «Перезапустить».'; \
         echo '==============================='; echo; \
         bash {} ; ec=$?; if [ \"$ec\" = 0 ]; then : > '{}'; fi; \
         echo; echo \"Exit: $ec\"; echo 'Press Enter to close'; read",
        path.display(),
        ready_flag.display()
    );
    let args = terminal_args(terminal, &bash_cmd);

    std::process::Command::new(terminal)
        .args(&args)
        .spawn()
        .map_err(|e| format!("Запуск {terminal}: {e}"))?;

    Ok(())
}

/// Windows: скачивает .exe-установщик последнего релиза во временный файл и
/// запускает его (NSIS-визард). Аналог Linux install.sh, но без терминала и
/// sudo — NSIS currentUser ставит без прав администратора. После успешного
/// запуска вызывающий код выходит из приложения, чтобы установщик мог заменить
/// файлы запущенного процесса. Заменяет прежнее поведение «открыть страницу
/// релиза в браузере» — обычному пользователю проще «нажал → обновилось».
#[cfg(target_os = "windows")]
async fn apply_update_windows() -> Result<(), String> {
    // 1. URL .exe-asset'а свежего релиза (без ETag-кэша — нужен актуальный сейчас).
    let url = updater::fetch_installable_asset_url()
        .await
        .map_err(|e| format!("Не удалось получить релиз: {e}"))?
        .ok_or_else(|| "В релизе нет готового установщика .exe".to_string())?;

    // 2. Скачиваем установщик во временный .exe (download-логика вынесена ниже).
    let path = download_installer_to_temp(&url).await?;

    // 3. Запускаем установщик (независимый процесс — переживёт выход приложения).
    std::process::Command::new(&path)
        .spawn()
        .map_err(|e| format!("Запуск установщика: {e}"))?;

    tracing::info!("Windows: установщик скачан и запущен ({})", path.display());
    Ok(())
}

/// Скачивает установщик по `url` целиком в память и сохраняет во временный
/// `.exe` в `%TEMP%`. Возвращает путь к файлу. `.keep()` оставляет файл после
/// drop, чтобы запущенный установщик успел его прочитать (очистится позже самим
/// `%TEMP%`). Таймаут щедрый — установщик ~7+ МБ.
#[cfg(target_os = "windows")]
async fn download_installer_to_temp(url: &str) -> Result<std::path::PathBuf, String> {
    use std::io::Write;

    let bytes = reqwest::Client::builder()
        .user_agent(format!("arcanaglyph/{}", updater::APP_VERSION))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Скачивание установщика: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Установщик HTTP: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Чтение установщика: {e}"))?;

    let tmp = tempfile::Builder::new()
        .prefix("arcanaglyph-update-")
        .suffix(".exe")
        .tempfile()
        .map_err(|e| format!("tempfile: {e}"))?;
    tmp.as_file()
        .write_all(&bytes)
        .map_err(|e| format!("Запись установщика: {e}"))?;
    tmp.into_temp_path().keep().map_err(|e| format!("temp keep: {e}"))
}

/// Путь к флаг-файлу «установка успешно завершена». Терминальная обёртка
/// (`apply_update_inner`) создаёт его при exit 0 install.sh; UI ждёт его перед
/// разблокировкой «Перезапустить». Кроссплатформенно (temp_dir), но реально
/// пишется только на Linux — на Windows `apply_update` выходит из приложения.
fn update_ready_flag_path() -> std::path::PathBuf {
    std::env::temp_dir().join("arcanaglyph-update-ready")
}

/// Tauri-команда: завершилась ли установка обновления успехом. UI в
/// applying-режиме держит «Перезапустить» disabled, пока здесь не вернётся
/// `true` (флаг от терминальной обёртки) — чтобы не перезапустить старую
/// версию до конца установки (баг #3 из UPDATER-UX-BUGS).
#[tauri::command]
pub fn update_install_ready() -> bool {
    update_ready_flag_path().exists()
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
