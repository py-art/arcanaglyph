// crates/arcanaglyph-app/src/commands/widget_ext.rs
//
// GNOME shell extension для виджета записи: установка в `~/.local/share/gnome-shell/extensions/`,
// включение через gsettings (минуя `gnome-extensions enable`, который без relogin
// не активирует свежескопированное расширение), выключение, проверка статуса,
// request_logout.

/// UUID нашего GNOME-расширения для виджета записи.
pub(crate) const WIDGET_EXT_UUID: &str = "arcanaglyph-widget@arfi.tech";

/// Источник файлов расширения (production: /usr/share/arcanaglyph/extension/...,
/// dev: <repo_root>/extension/...). Возвращаем первый существующий путь.
fn widget_ext_source_dir() -> Option<std::path::PathBuf> {
    // 1. Production-путь (.deb положил сюда)
    let prod = std::path::PathBuf::from("/usr/share/arcanaglyph/extension").join(WIDGET_EXT_UUID);
    if prod.is_dir() {
        return Some(prod);
    }
    // 2. Dev-путь относительно бинаря: target/{debug,release}/arcanaglyph → ../../extension/<uuid>
    if let Ok(exe) = std::env::current_exe()
        && let Some(target_dir) = exe.parent()
        && let Some(target_root) = target_dir.parent()
        && let Some(repo_root) = target_root.parent()
    {
        let dev = repo_root.join("extension").join(WIDGET_EXT_UUID);
        if dev.is_dir() {
            return Some(dev);
        }
    }
    // 3. Dev fallback: текущая рабочая директория (когда запущено `cargo run` из корня репо)
    if let Ok(cwd) = std::env::current_dir() {
        let dev = cwd.join("extension").join(WIDGET_EXT_UUID);
        if dev.is_dir() {
            return Some(dev);
        }
    }
    None
}

/// Целевой user-путь куда устанавливаем расширение.
fn widget_ext_user_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("gnome-shell/extensions").join(WIDGET_EXT_UUID))
}

/// Статус расширения: установлено ли в user dir и включено ли в gsettings.
#[derive(serde::Serialize)]
pub struct WidgetExtensionStatus {
    available: bool, // есть ли .so/файлы расширения в источнике (можем установить)
    installed: bool, // скопировано в ~/.local/share/gnome-shell/extensions/...
    enabled: bool,   // в gsettings org.gnome.shell enabled-extensions
}

#[tauri::command]
pub fn widget_extension_status() -> WidgetExtensionStatus {
    // GNOME-расширение есть только на Linux. На Windows/macOS возвращаем all-false:
    // фронтенд скрывает ряд при available=false (settings.ts), а gsettings-пробу
    // тут делать незачем — её в системе нет (избегаем заведомо падающего спавна).
    #[cfg(not(target_os = "linux"))]
    {
        return WidgetExtensionStatus {
            available: false,
            installed: false,
            enabled: false,
        };
    }
    #[cfg(target_os = "linux")]
    {
        widget_extension_status_linux()
    }
}

/// Linux-реализация статуса расширения (gsettings + проверка файлов).
#[cfg(target_os = "linux")]
fn widget_extension_status_linux() -> WidgetExtensionStatus {
    let available = widget_ext_source_dir().is_some();
    let installed = widget_ext_user_dir()
        .map(|p| p.join("metadata.json").is_file())
        .unwrap_or(false);
    // Читаем enabled-extensions через gsettings — самый надёжный способ.
    // Если gsettings нет (не GNOME) — enabled=false.
    let enabled = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.shell", "enabled-extensions"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.contains(WIDGET_EXT_UUID))
        .unwrap_or(false);
    WidgetExtensionStatus {
        available,
        installed,
        enabled,
    }
}

/// Установить и включить расширение виджета. Идемпотентно.
///
/// 1. Сравнивает version из metadata.json (src vs dst). Совпадает → пропускаем
///    копирование (файлы уже актуальны).
/// 2. Перекомпилирует gschemas только если копировали.
/// 3. Добавляет UUID в gsettings org.gnome.shell enabled-extensions
///    напрямую — `gnome-extensions enable` отказывается активировать
///    свежескопированное расширение пока gnome-shell не пере-сканировал
///    директорию (на Wayland без relogin'а это не происходит). gsettings
///    модификация идемпотентна (см. set_extension_enabled).
/// 4. Best-effort `gnome-extensions enable` для случая когда shell уже знает
///    об extension.
///
/// Возвращает `true` если расширение УЖЕ было активным до вызова — frontend
/// тогда показывает короткий toast без модала про logout.
#[tauri::command]
pub fn install_widget_extension() -> Result<bool, String> {
    let src = widget_ext_source_dir().ok_or_else(|| {
        "Файлы расширения не найдены ни в /usr/share/arcanaglyph/extension/, ни в <repo>/extension/".to_string()
    })?;
    let dst = widget_ext_user_dir().ok_or_else(|| "Не удалось определить XDG_DATA_HOME".to_string())?;

    let was_already_enabled = is_extension_enabled(WIDGET_EXT_UUID).unwrap_or(false);

    // Если в установленной копии та же version — файлы не трогаем.
    let needs_copy = !ext_version_matches(&src, &dst);
    if needs_copy {
        if dst.exists() {
            std::fs::remove_dir_all(&dst).map_err(|e| format!("Не удалось очистить {}: {}", dst.display(), e))?;
        }
        std::fs::create_dir_all(&dst).map_err(|e| format!("mkdir {}: {}", dst.display(), e))?;
        copy_dir_recursive(&src, &dst).map_err(|e| format!("copy {}→{}: {}", src.display(), dst.display(), e))?;
        let schemas_dir = dst.join("schemas");
        if schemas_dir.is_dir() {
            let _ = std::process::Command::new("glib-compile-schemas")
                .arg(&schemas_dir)
                .output();
        }
        tracing::info!("Файлы расширения скопированы в {}", dst.display());
    } else {
        tracing::info!("Файлы расширения уже актуальны, пропускаю копирование");
    }

    // gsettings: идемпотентно (set_extension_enabled сам проверяет состояние).
    set_extension_enabled(WIDGET_EXT_UUID, true)?;

    // Best-effort активация в текущей shell-сессии.
    let _ = std::process::Command::new("gnome-extensions")
        .args(["enable", WIDGET_EXT_UUID])
        .output();

    Ok(was_already_enabled)
}

/// Сравнивает поле `"version"` в metadata.json src и dst. true если оба
/// валидно прочитались и совпадают.
fn ext_version_matches(src: &std::path::Path, dst: &std::path::Path) -> bool {
    let read_ver = |p: &std::path::Path| -> Option<u64> {
        let txt = std::fs::read_to_string(p.join("metadata.json")).ok()?;
        let json: serde_json::Value = serde_json::from_str(&txt).ok()?;
        json.get("version").and_then(|v| v.as_u64())
    };
    match (read_ver(src), read_ver(dst)) {
        (Some(s), Some(d)) => s == d,
        _ => false,
    }
}

/// Проверяет, есть ли UUID в enabled-extensions через gsettings.
fn is_extension_enabled(uuid: &str) -> Result<bool, String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.shell", "enabled-extensions"])
        .output()
        .map_err(|e| format!("gsettings get: {}", e))?;
    if !out.status.success() {
        return Ok(false);
    }
    let cur = String::from_utf8_lossy(&out.stdout);
    Ok(parse_gvariant_strings(&cur).iter().any(|s| s == uuid))
}

/// Выключить расширение: убрать UUID из enabled-extensions через gsettings
/// (файлы оставляем — быстрое включение обратно). Best-effort `gnome-extensions
/// disable` для случая когда extension реально загружено в shell.
#[tauri::command]
pub fn disable_widget_extension() -> Result<(), String> {
    set_extension_enabled(WIDGET_EXT_UUID, false)?;
    let _ = std::process::Command::new("gnome-extensions")
        .args(["disable", WIDGET_EXT_UUID])
        .output();
    Ok(())
}

/// Парсит GVariant string array `['a', 'b']` → Vec<String>. Пустой массив
/// `@as []` или `[]` → пустой вектор.
fn parse_gvariant_strings(s: &str) -> Vec<String> {
    s.split('\'')
        .skip(1)
        .step_by(2)
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn format_gvariant_strings(list: &[String]) -> String {
    let inner = list.iter().map(|s| format!("'{}'", s)).collect::<Vec<_>>().join(", ");
    format!("[{}]", inner)
}

/// Atomic-ish modify: читает gsettings enabled-extensions, добавляет/убирает
/// UUID, пишет обратно. Если состояние уже правильное — no-op.
fn set_extension_enabled(uuid: &str, enable: bool) -> Result<(), String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.shell", "enabled-extensions"])
        .output()
        .map_err(|e| format!("gsettings get: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "gsettings get вернул ошибку: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let cur = String::from_utf8_lossy(&out.stdout);
    let mut list = parse_gvariant_strings(&cur);
    let already = list.iter().any(|s| s == uuid);
    if enable && !already {
        list.push(uuid.to_string());
    } else if !enable && already {
        list.retain(|s| s != uuid);
    } else {
        return Ok(());
    }
    let new_val = format_gvariant_strings(&list);
    let out = std::process::Command::new("gsettings")
        .args(["set", "org.gnome.shell", "enabled-extensions", &new_val])
        .output()
        .map_err(|e| format!("gsettings set: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "gsettings set вернул ошибку: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Запросить выход из GNOME-сессии. GNOME сам покажет диалог подтверждения о
/// несохранённой работе.
#[tauri::command]
pub fn request_logout() -> Result<(), String> {
    std::process::Command::new("gnome-session-quit")
        .args(["--logout"])
        .spawn()
        .map_err(|e| format!("gnome-session-quit: {}", e))?;
    Ok(())
}

/// Рекурсивное копирование директории (без external крейтов).
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}
