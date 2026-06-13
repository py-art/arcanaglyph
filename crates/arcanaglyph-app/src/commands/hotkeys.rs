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

/// Классифицирует значение gsettings binding как «пусто/нужно регистрировать».
/// Пустая строка, `''` (пустой GVariant) или текст ошибки `No such` → true.
/// Чистая функция (вынесена из `run_setup` ради тестируемости без tauri).
#[cfg(target_os = "linux")]
pub(crate) fn binding_is_empty(value: &str) -> bool {
    value.is_empty() || value == "''" || value.contains("No such")
}

/// Парсит вывод `setxkbmap -query` → `(layout, variant)`.
/// Отсутствующие поля → пустые строки. Чистая функция.
#[cfg(target_os = "linux")]
pub(crate) fn parse_setxkbmap_query(stdout: &str) -> (String, String) {
    let mut layout = String::new();
    let mut variant = String::new();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("layout:") {
            layout = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("variant:") {
            variant = rest.trim().to_string();
        }
    }
    (layout, variant)
}

/// Собирает argv для повторного `setxkbmap` (триггерит XKB-reload).
/// `variant` добавляется только если непустой. Чистая функция.
#[cfg(target_os = "linux")]
pub(crate) fn build_setxkbmap_args(layout: &str, variant: &str) -> Vec<String> {
    let mut args = vec!["-layout".to_string(), layout.to_string()];
    if !variant.is_empty() {
        args.push("-variant".to_string());
        args.push(variant.to_string());
    }
    args
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

/// Кириллический дубль gsettings-binding'а: берёт keysym после последнего `>`,
/// мапит латиницу→кириллицу, склеивает обратно с модификаторами. `None` — если
/// для клавиши нет кириллического дубля. Чистая функция (тестируема).
#[cfg(target_os = "linux")]
fn cyrillic_binding(binding: &str) -> Option<String> {
    let key = binding.rsplit('>').next().unwrap_or("");
    let cyrillic = latin_to_cyrillic_keysym(key)?;
    let mods = &binding[..binding.len() - key.len()];
    Some(format!("{}{}", mods, cyrillic))
}

/// Парсит gsettings-список путей `['p1', 'p2']` (или `@as []` / пусто) в `Vec`.
/// Чистая функция — общий парсер для register и check_hotkey_conflict.
#[cfg(target_os = "linux")]
fn parse_gsettings_paths(raw: &str) -> Vec<String> {
    if raw == "@as []" || raw.is_empty() {
        return vec![];
    }
    raw.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|s| s.trim().trim_matches('\'').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Базовая schema custom-keybinding'а GNOME (per-path).
#[cfg(target_os = "linux")]
const KEYBINDING_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";

/// `gsettings set <schema>:<path> <key> <val>`. Ошибка только при сбое запуска
/// процесса (не при non-zero exit) — как и было в исходном inline-замыкании.
#[cfg(target_os = "linux")]
fn gs_set(path: &str, key: &str, val: &str) -> Result<(), String> {
    let schema_path = format!("{}:{}", KEYBINDING_SCHEMA, path);
    std::process::Command::new("gsettings")
        .args(["set", &schema_path, key, val])
        .output()
        .map_err(|e| format!("gsettings set {} {}: {}", key, val, e))?;
    Ok(())
}

/// Регистрирует пару слотов (латинский + кириллический дубль) одного хоткея.
/// Пустой `hotkey` → no-op. Кириллический слот создаётся только если у клавиши
/// есть кириллический дубль. Вынесено из `register_gnome_hotkeys_linux` —
/// убирает дублирование trigger/pause-блоков.
#[cfg(target_os = "linux")]
fn register_hotkey_pair(
    hotkey: &str,
    latin_path: &str,
    cyr_path: &str,
    name: &str,
    name_ru: &str,
    command: &str,
) -> Result<(), String> {
    if hotkey.is_empty() {
        return Ok(());
    }
    let binding = tauri_hotkey_to_gsettings(hotkey);
    gs_set(latin_path, "name", &format!("'{}'", name))?;
    gs_set(latin_path, "command", &format!("'{}'", command))?;
    gs_set(latin_path, "binding", &format!("'{}'", binding))?;

    if let Some(cyr_binding) = cyrillic_binding(&binding) {
        gs_set(cyr_path, "name", &format!("'{}'", name_ru))?;
        gs_set(cyr_path, "command", &format!("'{}'", command))?;
        gs_set(cyr_path, "binding", &format!("'{}'", cyr_binding))?;
    }
    Ok(())
}

/// Добавляет в `paths` слоты (латинский + кириллический, если применим) для
/// хоткея, если их там ещё нет. Пустой `hotkey` → no-op.
#[cfg(target_os = "linux")]
fn append_keybinding_paths(paths: &mut Vec<String>, hotkey: &str, latin_path: &str, cyr_path: &str) {
    if hotkey.is_empty() {
        return;
    }
    if !paths.iter().any(|p| p == latin_path) {
        paths.push(latin_path.to_string());
    }
    let binding = tauri_hotkey_to_gsettings(hotkey);
    if cyrillic_binding(&binding).is_some() && !paths.iter().any(|p| p == cyr_path) {
        paths.push(cyr_path.to_string());
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

/// Встроенные схемы GNOME, сканируемые на занятость комбинации.
#[cfg(target_os = "linux")]
const BUILTIN_KEYBINDING_SCHEMAS: [&str; 3] = [
    "org.gnome.desktop.wm.keybindings",
    "org.gnome.shell.keybindings",
    "org.gnome.mutter.keybindings",
];

/// Оркестратор: пустой хоткей или пустой binding → нет конфликта; иначе
/// сначала встроенные схемы, затем custom-keybindings. Логика вынесена в
/// тонкие I/O-обёртки + чистые матчеры (тестируемы).
#[cfg(target_os = "linux")]
fn check_hotkey_conflict_gnome(hotkey: String) -> Result<Option<String>, String> {
    if hotkey.is_empty() {
        return Ok(None);
    }
    let binding = tauri_hotkey_to_gsettings(&hotkey);
    if binding.is_empty() {
        return Ok(None);
    }
    if let Some(conflict) = scan_builtin_keybinding_schemas(&binding) {
        return Ok(Some(conflict));
    }
    scan_custom_keybinding_conflict(&binding)
}

/// Снимает обёртку gsettings-вывода: trim + strip `'…'`. Чистая (тестируема).
#[cfg(target_os = "linux")]
fn unquote_gsettings_value(raw: &str) -> String {
    raw.trim().trim_matches('\'').to_string()
}

/// Чистый матчер: первая строка `list-recursively`, содержащая `binding`,
/// → имя настройки (второе слово, `???` если нет). Тестируема.
#[cfg(target_os = "linux")]
fn builtin_conflict_name(text: &str, binding: &str) -> Option<String> {
    text.lines()
        .find(|line| line.contains(binding))
        .map(|line| line.split_whitespace().nth(1).unwrap_or("???").to_string())
}

/// Сканирует встроенные схемы на занятость `binding`. Сбой запуска gsettings
/// для одной схемы → пропуск (как было в исходном inline-цикле), не ошибка.
#[cfg(target_os = "linux")]
fn scan_builtin_keybinding_schemas(binding: &str) -> Option<String> {
    for schema in &BUILTIN_KEYBINDING_SCHEMAS {
        let Ok(out) = std::process::Command::new("gsettings")
            .args(["list-recursively", schema])
            .output()
        else {
            continue;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        if let Some(name) = builtin_conflict_name(&text, binding) {
            return Some(format!("{} ({})", name, schema));
        }
    }
    None
}

/// Проверяет custom-keybindings (кроме наших `arcanaglyph-*`) на занятость
/// `binding`. Ошибка только при сбое первичного `get custom-keybindings`.
#[cfg(target_os = "linux")]
fn scan_custom_keybinding_conflict(binding: &str) -> Result<Option<String>, String> {
    let output = std::process::Command::new("gsettings")
        .args([
            "get",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    let paths_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if paths_str == "@as []" || paths_str.is_empty() {
        return Ok(None);
    }
    for path in parse_gsettings_paths(&paths_str) {
        // Пропускаем наши собственные слоты
        if path.contains("arcanaglyph-") {
            continue;
        }
        if let Some(name) = custom_path_conflict_name(&path, binding) {
            return Ok(Some(format!("{} (custom keybinding)", name)));
        }
    }
    Ok(None)
}

/// Для одного custom-path: если его `binding` совпадает с искомым — вернуть
/// `name` слота (`???` если не прочиталось). Сбой gsettings → `None` (пропуск).
#[cfg(target_os = "linux")]
fn custom_path_conflict_name(path: &str, binding: &str) -> Option<String> {
    let schema_path = format!("{}:{}", KEYBINDING_SCHEMA, path);
    let out = std::process::Command::new("gsettings")
        .args(["get", &schema_path, "binding"])
        .output()
        .ok()?;
    let existing = unquote_gsettings_value(&String::from_utf8_lossy(&out.stdout));
    if existing != binding {
        return None;
    }
    let name = std::process::Command::new("gsettings")
        .args(["get", &schema_path, "name"])
        .output()
        .map(|o| unquote_gsettings_value(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_else(|_| "???".to_string());
    Some(name)
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

    // Слоты ArcanaGlyph: латинский + кириллический дубль для trigger и pause.
    let ag_trigger_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-trigger/";
    let ag_trigger_cyr_path =
        "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-trigger-cyr/";
    let ag_pause_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-pause/";
    let ag_pause_cyr_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/arcanaglyph-pause-cyr/";

    // Определяем путь к скриптам
    let scripts_dir =
        CoreConfig::scripts_dir().ok_or_else(|| "Не удалось определить директорию скриптов".to_string())?;
    let trigger_cmd = scripts_dir.join("ag-trigger").display().to_string();
    let pause_cmd = scripts_dir.join("ag-pause").display().to_string();

    // Регистрируем слоты (латиница + кириллический дубль для русской раскладки).
    register_hotkey_pair(
        &hotkey_trigger,
        ag_trigger_path,
        ag_trigger_cyr_path,
        "ArcanaGlyph Trigger",
        "ArcanaGlyph Trigger (RU)",
        &trigger_cmd,
    )?;
    register_hotkey_pair(
        &hotkey_pause,
        ag_pause_path,
        ag_pause_cyr_path,
        "ArcanaGlyph Pause",
        "ArcanaGlyph Pause (RU)",
        &pause_cmd,
    )?;

    // Обновляем список custom-keybindings — добавляем наши пути если их нет.
    let mut paths = parse_gsettings_paths(&current);
    append_keybinding_paths(&mut paths, &hotkey_trigger, ag_trigger_path, ag_trigger_cyr_path);
    append_keybinding_paths(&mut paths, &hotkey_pause, ag_pause_path, ag_pause_cyr_path);

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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn test_tauri_hotkey_to_gsettings() {
        // Пустой хоткей → пустая строка (не задан).
        assert_eq!(tauri_hotkey_to_gsettings(""), "");
        // Backtick → grave (главный кейс: Ctrl+Ё).
        assert_eq!(tauri_hotkey_to_gsettings("Control+`"), "<Control>grave");
        // Полный набор модификаторов + именованная клавиша (lowercase).
        assert_eq!(
            tauri_hotkey_to_gsettings("Super+Alt+Control+Space"),
            "<Super><Alt><Control>space"
        );
        // Shift + буква.
        assert_eq!(tauri_hotkey_to_gsettings("Shift+A"), "<Shift>a");
        // Одна клавиша без модификаторов.
        assert_eq!(tauri_hotkey_to_gsettings("F1"), "f1");
    }

    #[test]
    fn test_latin_to_cyrillic_keysym() {
        // Раскладочные дубли: латинская клавиша → кириллический keysym.
        assert_eq!(latin_to_cyrillic_keysym("q"), Some("Cyrillic_shorti"));
        assert_eq!(latin_to_cyrillic_keysym("a"), Some("Cyrillic_ef"));
        assert_eq!(latin_to_cyrillic_keysym("m"), Some("Cyrillic_softsign"));
        // Спец-кейс Ё: grave → Cyrillic_io (иначе Ctrl+Ё не ловится на ru-раскладке).
        assert_eq!(latin_to_cyrillic_keysym("grave"), Some("Cyrillic_io"));
        // Неизвестная клавиша → None (нет кириллического дубля).
        assert_eq!(latin_to_cyrillic_keysym("1"), None);
        assert_eq!(latin_to_cyrillic_keysym("space"), None);
    }

    #[test]
    fn test_cyrillic_binding() {
        // Ctrl+grave → кириллический дубль с теми же модификаторами.
        assert_eq!(
            cyrillic_binding("<Control>grave").as_deref(),
            Some("<Control>Cyrillic_io")
        );
        // Несколько модификаторов сохраняются.
        assert_eq!(
            cyrillic_binding("<Super><Alt>q").as_deref(),
            Some("<Super><Alt>Cyrillic_shorti")
        );
        // Клавиша без кириллического дубля → None.
        assert_eq!(cyrillic_binding("<Control>space"), None);
        assert_eq!(cyrillic_binding("f1"), None);
    }

    #[test]
    fn test_parse_gsettings_paths() {
        // Пустой список GVariant и пустая строка → пусто.
        assert!(parse_gsettings_paths("@as []").is_empty());
        assert!(parse_gsettings_paths("").is_empty());
        // Один путь.
        assert_eq!(parse_gsettings_paths("['/a/b/']"), vec!["/a/b/".to_string()]);
        // Несколько путей с пробелами и кавычками.
        assert_eq!(
            parse_gsettings_paths("['/a/', '/b/', '/c/']"),
            vec!["/a/".to_string(), "/b/".to_string(), "/c/".to_string()]
        );
    }

    #[test]
    fn test_binding_is_empty() {
        assert!(binding_is_empty(""));
        assert!(binding_is_empty("''"));
        assert!(binding_is_empty("No such schema"));
        assert!(!binding_is_empty("'<Control>grave'"));
    }

    #[test]
    fn test_parse_setxkbmap_query() {
        let out = "rules:      evdev\nmodel:      pc105\nlayout:     us,ru\nvariant:    ,\n";
        let (layout, variant) = parse_setxkbmap_query(out);
        assert_eq!(layout, "us,ru");
        assert_eq!(variant, ",");
    }

    #[test]
    fn test_build_setxkbmap_args() {
        assert_eq!(build_setxkbmap_args("us,ru", ""), ["-layout", "us,ru"]);
        assert_eq!(
            build_setxkbmap_args("us,ru", "colemak"),
            ["-layout", "us,ru", "-variant", "colemak"]
        );
    }

    #[test]
    fn test_unquote_gsettings_value() {
        // Снимает пробелы-обёртку и одинарные кавычки.
        assert_eq!(unquote_gsettings_value("  '<Control>grave'  "), "<Control>grave");
        // Без кавычек — только trim.
        assert_eq!(unquote_gsettings_value(" foo "), "foo");
        // Пустой вывод.
        assert_eq!(unquote_gsettings_value(""), "");
    }

    #[test]
    fn test_builtin_conflict_name() {
        let text = "org.gnome.desktop.wm.keybindings switch-windows ['<Control>grave']\n\
                    org.gnome.desktop.wm.keybindings close ['<Alt>F4']";
        // Совпадение по binding → второе слово строки (имя настройки).
        assert_eq!(
            builtin_conflict_name(text, "<Control>grave").as_deref(),
            Some("switch-windows")
        );
        // Нет совпадения → None.
        assert_eq!(builtin_conflict_name(text, "<Super>p"), None);
        // Строка без второго слова → "???".
        assert_eq!(
            builtin_conflict_name("<Control>grave", "<Control>grave").as_deref(),
            Some("???")
        );
    }
}
