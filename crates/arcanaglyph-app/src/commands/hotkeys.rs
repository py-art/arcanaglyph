// crates/arcanaglyph-app/src/commands/hotkeys.rs
//
// GNOME custom-keybindings через gsettings: проверка конфликтов, регистрация
// trigger/pause и кириллических дублей. На Wayland tauri-plugin-global-shortcut
// не работает, на X11+GNOME mutter перехватывает event'ы — это единственный
// надёжный путь. На Win/macOS — no-op.

#[cfg(target_os = "linux")]
use arcanaglyph_core::CoreConfig;

/// Конвертация формата хоткея из Tauri ("Super+Alt+Control+Space") в gsettings ("<Super><Alt><Control>space")
#[cfg(target_os = "linux")]
pub(crate) fn tauri_hotkey_to_gsettings(hotkey: &str) -> String {
    if hotkey.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = hotkey.split('+').collect();
    let mut mods = String::new();
    let mut key = String::new();
    for part in &parts {
        match *part {
            "Super" => mods.push_str("<Super>"),
            "Alt" => mods.push_str("<Alt>"),
            "Control" => mods.push_str("<Control>"),
            "Shift" => mods.push_str("<Shift>"),
            "`" => key = "grave".to_string(),
            k => key = k.to_lowercase(),
        }
    }
    format!("{}{}", mods, key)
}

/// Маппинг латинских клавиш → XKB keysym кириллических (для GNOME gsettings)
#[cfg(target_os = "linux")]
fn latin_to_cyrillic_keysym(key: &str) -> Option<&'static str> {
    match key {
        "q" => Some("Cyrillic_shorti"),   // й
        "w" => Some("Cyrillic_tse"),      // ц
        "e" => Some("Cyrillic_u"),        // у
        "r" => Some("Cyrillic_ka"),       // к
        "t" => Some("Cyrillic_ie"),       // е
        "y" => Some("Cyrillic_en"),       // н
        "u" => Some("Cyrillic_ghe"),      // г
        "i" => Some("Cyrillic_sha"),      // ш
        "o" => Some("Cyrillic_shcha"),    // щ
        "p" => Some("Cyrillic_ze"),       // з
        "a" => Some("Cyrillic_ef"),       // ф
        "s" => Some("Cyrillic_yeru"),     // ы
        "d" => Some("Cyrillic_ve"),       // в
        "f" => Some("Cyrillic_a"),        // а
        "g" => Some("Cyrillic_pe"),       // п
        "h" => Some("Cyrillic_er"),       // р
        "j" => Some("Cyrillic_o"),        // о
        "k" => Some("Cyrillic_el"),       // л
        "l" => Some("Cyrillic_de"),       // д
        "z" => Some("Cyrillic_ya"),       // я
        "x" => Some("Cyrillic_che"),      // ч
        "c" => Some("Cyrillic_es"),       // с
        "v" => Some("Cyrillic_em"),       // м
        "b" => Some("Cyrillic_i"),        // и
        "n" => Some("Cyrillic_te"),       // т
        "m" => Some("Cyrillic_softsign"), // ь
        // Спец-клавиша: на русской раскладке клавиша слева от 1 (`/~) даёт Ё.
        // Без этого маппинга keybinding `<Control>grave` не срабатывает на ru-раскладке —
        // GNOME ищет `<Control>Cyrillic_io` и не находит его.
        "grave" => Some("Cyrillic_io"), // Ё
        _ => None,
    }
}

/// Tauri-команда: проверить, занята ли комбинация клавиш в GNOME.
/// На Win/macOS возвращает None — там GNOME отсутствует.
#[tauri::command]
pub fn check_hotkey_conflict(hotkey: String) -> Result<Option<String>, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = hotkey;
        return Ok(None);
    }
    #[cfg(target_os = "linux")]
    check_hotkey_conflict_gnome(hotkey)
}

#[cfg(target_os = "linux")]
fn check_hotkey_conflict_gnome(hotkey: String) -> Result<Option<String>, String> {
    if hotkey.is_empty() {
        return Ok(None);
    }
    let binding = tauri_hotkey_to_gsettings(&hotkey);
    if binding.is_empty() {
        return Ok(None);
    }

    // Сканируем все схемы GNOME на совпадение
    let schemas = [
        "org.gnome.desktop.wm.keybindings",
        "org.gnome.shell.keybindings",
        "org.gnome.mutter.keybindings",
    ];

    for schema in &schemas {
        let output = std::process::Command::new("gsettings")
            .args(["list-recursively", schema])
            .output();
        if let Ok(out) = output {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if line.contains(&binding) {
                    // Извлекаем имя настройки (второе слово в строке)
                    let name = line.split_whitespace().nth(1).unwrap_or("???");
                    return Ok(Some(format!("{} ({})", name, schema)));
                }
            }
        }
    }

    // Проверяем custom keybindings (кроме наших arcanaglyph-*)
    let output = std::process::Command::new("gsettings")
        .args([
            "get",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    let paths_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if paths_str != "@as []" && !paths_str.is_empty() {
        let paths: Vec<String> = paths_str
            .trim_matches(|c| c == '[' || c == ']')
            .split(',')
            .map(|s| s.trim().trim_matches('\'').trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let base = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
        for path in &paths {
            // Пропускаем наши собственные слоты
            if path.contains("arcanaglyph-") {
                continue;
            }
            let schema_path = format!("{}:{}", base, path);
            let out = std::process::Command::new("gsettings")
                .args(["get", &schema_path, "binding"])
                .output();
            if let Ok(out) = out {
                let existing = String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .trim_matches('\'')
                    .to_string();
                if existing == binding {
                    let name_out = std::process::Command::new("gsettings")
                        .args(["get", &schema_path, "name"])
                        .output();
                    let name = name_out
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_string())
                        .unwrap_or_else(|_| "???".to_string());
                    return Ok(Some(format!("{} (custom keybinding)", name)));
                }
            }
        }
    }

    Ok(None)
}

/// Tauri-команда: зарегистрировать глобальные хоткеи через gsettings (Wayland/GNOME).
/// На Win/macOS — no-op.
#[tauri::command]
pub fn register_gnome_hotkeys(hotkey_trigger: String, hotkey_pause: String) -> Result<(), String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (hotkey_trigger, hotkey_pause);
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    register_gnome_hotkeys_linux(hotkey_trigger, hotkey_pause)
}

#[cfg(target_os = "linux")]
fn register_gnome_hotkeys_linux(hotkey_trigger: String, hotkey_pause: String) -> Result<(), String> {
    // Получаем текущий список custom keybindings
    let output = std::process::Command::new("gsettings")
        .args([
            "get",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
        ])
        .output()
        .map_err(|e| format!("Не удалось вызвать gsettings: {}", e))?;
    let current = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Определяем слоты для ArcanaGlyph (ищем существующие или берём свободные)
    let ag_trigger_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-trigger/";
    let ag_trigger_cyr_path =
        "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-trigger-cyr/";
    let ag_pause_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-pause/";
    let ag_pause_cyr_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-pause-cyr/";
    let base = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";

    // Вспомогательная функция для выполнения gsettings set
    let gs_set = |path: &str, key: &str, val: &str| -> Result<(), String> {
        let schema_path = format!("{}:{}", base, path);
        std::process::Command::new("gsettings")
            .args(["set", &schema_path, key, val])
            .output()
            .map_err(|e| format!("gsettings set {} {}: {}", key, val, e))?;
        Ok(())
    };

    // Определяем путь к скриптам
    let scripts_dir =
        CoreConfig::scripts_dir().ok_or_else(|| "Не удалось определить директорию скриптов".to_string())?;
    let trigger_cmd = scripts_dir.join("ag-trigger").display().to_string();
    let pause_cmd = scripts_dir.join("ag-pause").display().to_string();

    // Регистрируем trigger (латиница)
    if !hotkey_trigger.is_empty() {
        let binding = tauri_hotkey_to_gsettings(&hotkey_trigger);
        gs_set(ag_trigger_path, "name", "'ArcanaGlyph Trigger'")?;
        gs_set(ag_trigger_path, "command", &format!("'{}'", trigger_cmd))?;
        gs_set(ag_trigger_path, "binding", &format!("'{}'", binding))?;

        // Кириллический дубль (для русской раскладки)
        let key = binding.rsplit('>').next().unwrap_or("");
        if let Some(cyrillic) = latin_to_cyrillic_keysym(key) {
            let mods = &binding[..binding.len() - key.len()];
            let cyr_binding = format!("{}{}", mods, cyrillic);
            gs_set(ag_trigger_cyr_path, "name", "'ArcanaGlyph Trigger (RU)'")?;
            gs_set(ag_trigger_cyr_path, "command", &format!("'{}'", trigger_cmd))?;
            gs_set(ag_trigger_cyr_path, "binding", &format!("'{}'", cyr_binding))?;
        }
    }

    // Регистрируем pause (латиница)
    if !hotkey_pause.is_empty() {
        let binding = tauri_hotkey_to_gsettings(&hotkey_pause);
        gs_set(ag_pause_path, "name", "'ArcanaGlyph Pause'")?;
        gs_set(ag_pause_path, "command", &format!("'{}'", pause_cmd))?;
        gs_set(ag_pause_path, "binding", &format!("'{}'", binding))?;

        // Кириллический дубль
        let key = binding.rsplit('>').next().unwrap_or("");
        if let Some(cyrillic) = latin_to_cyrillic_keysym(key) {
            let mods = &binding[..binding.len() - key.len()];
            let cyr_binding = format!("{}{}", mods, cyrillic);
            gs_set(ag_pause_cyr_path, "name", "'ArcanaGlyph Pause (RU)'")?;
            gs_set(ag_pause_cyr_path, "command", &format!("'{}'", pause_cmd))?;
            gs_set(ag_pause_cyr_path, "binding", &format!("'{}'", cyr_binding))?;
        }
    }

    // Обновляем список custom-keybindings — добавляем наши пути если их нет
    let mut paths: Vec<String> = if current == "@as []" || current.is_empty() {
        vec![]
    } else {
        // Парсим ['path1', 'path2', ...]
        current
            .trim_matches(|c| c == '[' || c == ']')
            .split(',')
            .map(|s| s.trim().trim_matches('\'').trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    if !hotkey_trigger.is_empty() && !paths.iter().any(|p| p == ag_trigger_path) {
        paths.push(ag_trigger_path.to_string());
    }
    // Кириллический дубль trigger
    let trigger_binding = tauri_hotkey_to_gsettings(&hotkey_trigger);
    let trigger_key = trigger_binding.rsplit('>').next().unwrap_or("");
    if !hotkey_trigger.is_empty()
        && latin_to_cyrillic_keysym(trigger_key).is_some()
        && !paths.iter().any(|p| p == ag_trigger_cyr_path)
    {
        paths.push(ag_trigger_cyr_path.to_string());
    }
    if !hotkey_pause.is_empty() && !paths.iter().any(|p| p == ag_pause_path) {
        paths.push(ag_pause_path.to_string());
    }
    // Кириллический дубль pause
    let pause_binding = tauri_hotkey_to_gsettings(&hotkey_pause);
    let pause_key = pause_binding.rsplit('>').next().unwrap_or("");
    if !hotkey_pause.is_empty()
        && latin_to_cyrillic_keysym(pause_key).is_some()
        && !paths.iter().any(|p| p == ag_pause_cyr_path)
    {
        paths.push(ag_pause_cyr_path.to_string());
    }

    let paths_str = format!(
        "[{}]",
        paths.iter().map(|p| format!("'{}'", p)).collect::<Vec<_>>().join(", ")
    );
    std::process::Command::new("gsettings")
        .args([
            "set",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
            &paths_str,
        ])
        .output()
        .map_err(|e| format!("Не удалось обновить список keybindings: {}", e))?;

    tracing::info!(
        "GNOME хоткеи зарегистрированы: trigger='{}', pause='{}'",
        hotkey_trigger,
        hotkey_pause
    );
    Ok(())
}
