// crates/arcanaglyph-app/src/setup/bootstrap.rs
//
// Подготовка окружения до старта Tauri: выбор libonnxruntime.so для ORT,
// установка g_prgname (WM_CLASS для GNOME Dash), autostart .desktop файл,
// установка Wayland-скриптов ag-trigger/ag-pause, CLI `arcanaglyph --grant-portal`.

#[cfg(target_os = "linux")]
use arcanaglyph_core::CoreConfig;

/// Управляет автозапуском через .desktop файл в ~/.config/autostart/
#[cfg(target_os = "linux")]
pub(crate) fn set_autostart(enabled: bool) {
    let home = match std::env::var("HOME") {
        Ok(h) => std::path::PathBuf::from(h),
        Err(_) => return,
    };
    let autostart_dir = home.join(".config/autostart");
    let desktop_file = autostart_dir.join("arcanaglyph.desktop");

    if enabled {
        let _ = std::fs::create_dir_all(&autostart_dir);

        // Определяем путь к исполняемому файлу
        let exec_path = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "arcanaglyph-app".to_string());

        let content = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=ArcanaGlyph\n\
             Comment=Голосовой ввод текста\n\
             Exec={}\n\
             Icon=arcanaglyph\n\
             Terminal=false\n\
             Categories=Utility;Audio;\n\
             X-GNOME-Autostart-enabled=true\n",
            exec_path
        );

        if let Err(e) = std::fs::write(&desktop_file, content) {
            tracing::warn!("Не удалось создать autostart: {}", e);
        } else {
            tracing::info!("Автозапуск включён: {}", desktop_file.display());
        }
    } else if desktop_file.exists() {
        let _ = std::fs::remove_file(&desktop_file);
        tracing::info!("Автозапуск отключён");
    }
}

// Заглушка автозапуска для Windows/macOS.
// На Windows нужен HKCU\Software\Microsoft\Windows\CurrentVersion\Run,
// на macOS — ~/Library/LaunchAgents/*.plist. Оставлено на следующий этап портирования.
#[cfg(not(target_os = "linux"))]
pub(crate) fn set_autostart(_enabled: bool) {}

/// Устанавливает UDP-скрипты ag-trigger и ag-pause (для Wayland)
#[cfg(target_os = "linux")]
pub(crate) fn install_wayland_scripts() {
    // Скрипты ставим на ЛЮБОМ Linux: на Wayland tauri-plugin-global-shortcut
    // вообще не работает (нет X11 grab), на X11+GNOME он часто не доставляет
    // event'ы (mutter перехватывает раньше). В обоих случаях нативные GNOME
    // custom-keybindings → ag-trigger → UDP — единственное что работает надёжно.
    let bin_dir = match CoreConfig::scripts_dir() {
        Some(d) => d,
        None => return,
    };
    let _ = std::fs::create_dir_all(&bin_dir);

    let scripts = [
        (
            "ag-trigger",
            "#!/bin/bash\n# ArcanaGlyph: UDP-триггер записи\necho \"trigger\" | /usr/bin/nc -u -w0 127.0.0.1 9002\n",
        ),
        (
            "ag-pause",
            "#!/bin/bash\n# ArcanaGlyph: UDP-триггер паузы\necho \"pause\" | /usr/bin/nc -u -w0 127.0.0.1 9002\n",
        ),
    ];

    for (name, content) in &scripts {
        let path = bin_dir.join(name);
        if !path.exists() {
            if let Err(e) = std::fs::write(&path, content) {
                tracing::warn!("Не удалось создать {}: {}", path.display(), e);
                continue;
            }
            // chmod +x
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
            tracing::info!("Установлен скрипт: {}", path.display());
        }
    }
}

// Заглушка установки Wayland-скриптов для Windows/macOS — там нет ни Wayland,
// ни /usr/bin/nc; UDP-триггер всё ещё доступен через прямую отправку датаграмм.
#[cfg(not(target_os = "linux"))]
pub(crate) fn install_wayland_scripts() {}

/// Выбирает путь к `libonnxruntime.so` для load-dynamic backend ORT и записывает его в
/// `ORT_DYLIB_PATH`. ВАЖНО: вызывать ДО первого касания `ort` (первый вызов —
/// `Session::builder()` в `gigaam/transcriber.rs`). Не имеет эффекта если ORT_DYLIB_PATH
/// уже выставлена (например, Makefile при `make run`).
///
/// Приоритет:
/// 1. `ORT_DYLIB_PATH` в env — оставляем как есть (dev override).
/// 2. `/usr/local/lib/libonnxruntime.so` — self-build пользователя (десктоп с самосборкой ORT).
/// 3. Bundled в `.deb` — `/usr/lib/arcanaglyph/libonnxruntime-{avx2,noavx}.so`,
///    выбор по runtime AVX-detection.
///
/// Если ничего не нашли — оставляем env пустой и ort попробует системный dlopen
/// (LD_LIBRARY_PATH, /usr/lib, /etc/ld.so.cache). Это путь dev-сборки на машине без
/// нашего pre-arrangement'а — fallback логика ничего не ломает.
#[cfg(target_os = "linux")]
pub fn setup_ort_dylib_path() {
    use std::path::Path;

    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        tracing::info!(
            "ORT_DYLIB_PATH = {} (взят из env)",
            std::env::var("ORT_DYLIB_PATH").unwrap_or_default()
        );
        return;
    }

    let local_lib = Path::new("/usr/local/lib/libonnxruntime.so");
    if local_lib.exists() {
        // SAFETY: вызывается в main() до спавна тредов, до загрузки ort.
        unsafe { std::env::set_var("ORT_DYLIB_PATH", local_lib) };
        tracing::info!("ORT_DYLIB_PATH = {} (self-build override)", local_lib.display());
        return;
    }

    #[cfg(target_arch = "x86_64")]
    let bundled = if std::is_x86_feature_detected!("avx") {
        "/usr/lib/arcanaglyph/libonnxruntime-avx2.so"
    } else {
        "/usr/lib/arcanaglyph/libonnxruntime-noavx.so"
    };
    #[cfg(not(target_arch = "x86_64"))]
    let bundled = "/usr/lib/arcanaglyph/libonnxruntime.so";

    let bundled_path = Path::new(bundled);
    if bundled_path.exists() {
        // SAFETY: вызывается в main() до спавна тредов, до загрузки ort.
        unsafe { std::env::set_var("ORT_DYLIB_PATH", bundled_path) };
        tracing::info!("ORT_DYLIB_PATH = {} (bundled .deb)", bundled_path.display());
        return;
    }

    tracing::warn!(
        "ORT_DYLIB_PATH не выставлена и libonnxruntime.so не найдена ни в /usr/local/lib, \
         ни в /usr/lib/arcanaglyph. ORT попробует загрузить через системный dlopen — \
         если в LD_LIBRARY_PATH нет нужной либы, GigaAM/Qwen3-ASR упадут при инициализации."
    );
}

/// Windows-аналог: выбирает `onnxruntime.dll` для load-dynamic backend ORT.
/// Зеркалит Linux-логику, но под раскладку NSIS-инсталлятора.
///
/// Приоритет:
/// 1. `ORT_DYLIB_PATH` в env — оставляем как есть (dev override / `cargo run`).
/// 2. `onnxruntime.dll` рядом с exe — стандартное место для DLL на Windows
///    (загрузчик и так нашёл бы её сам, но выставляем явно для предсказуемости).
/// 3. `libs/onnxruntime.dll` рядом с exe — раскладка Tauri-ресурса
///    (`bundle.resources = ["libs/onnxruntime.dll"]` в `tauri.windows.conf.json`).
/// 4. `resources/libs/onnxruntime.dll` — запасной вариант, если Tauri положит
///    ресурсы в поддиректорию `resources/`.
///
/// Если ничего не нашли — оставляем env пустой, ORT попробует загрузить DLL
/// из системного PATH (то же поведение, что dlopen на Linux).
#[cfg(target_os = "windows")]
pub fn setup_ort_dylib_path() {
    use std::path::PathBuf;

    if std::env::var_os("ORT_DYLIB_PATH").is_some() {
        tracing::info!(
            "ORT_DYLIB_PATH = {} (взят из env)",
            std::env::var("ORT_DYLIB_PATH").unwrap_or_default()
        );
        return;
    }

    let exe_dir = match std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)) {
        Some(dir) => dir,
        None => {
            tracing::warn!("Не удалось определить каталог exe — ORT_DYLIB_PATH не выставлена");
            return;
        }
    };

    let candidates = [
        exe_dir.join("onnxruntime.dll"),
        exe_dir.join("libs").join("onnxruntime.dll"),
        exe_dir.join("resources").join("libs").join("onnxruntime.dll"),
    ];

    for path in &candidates {
        if path.exists() {
            // SAFETY: вызывается в main() до спавна тредов, до загрузки ort.
            unsafe { std::env::set_var("ORT_DYLIB_PATH", path) };
            tracing::info!("ORT_DYLIB_PATH = {} (bundled рядом с exe)", path.display());
            return;
        }
    }

    tracing::warn!(
        "onnxruntime.dll не найдена рядом с exe (искал: onnxruntime.dll, libs/, \
         resources/libs/). ORT попробует загрузить через системный PATH — если \
         там её нет, GigaAM/Qwen3-ASR упадут при инициализации."
    );
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub fn setup_ort_dylib_path() {
    // На macOS ORT крейт ищет либу через системные механизмы — ничего не делаем.
}

/// Проставляет glib `g_prgname` в "arcanaglyph", чтобы GTK/GDK выставили `WM_CLASS`
/// в "arcanaglyph" вне зависимости от физического имени бинаря.
///
/// Зачем: в self-contained `.deb` у нас два бинаря — `arcanaglyph-avx` и
/// `arcanaglyph-noavx` (см. `assets/scripts/arcanaglyph-wrapper.sh`). По умолчанию
/// GTK берёт `g_prgname` из `argv[0]` → `WM_CLASS = "arcanaglyph-noavx"`. Это не
/// совпадает со `StartupWMClass=arcanaglyph` в `assets/arcanaglyph.desktop`, и
/// GNOME shell не привязывает работающее окно к ярлыку приложения — в Dash
/// появляется отдельная иконка с именем бинаря. После явной установки `g_prgname`
/// `WM_CLASS = "arcanaglyph"` и Dash корректно группирует окно с ярлыком.
///
/// Вызывать ДО любого GTK/GDK init (т.е. до `tauri::Builder::new()`).
#[cfg(target_os = "linux")]
pub fn setup_program_name() {
    unsafe extern "C" {
        fn g_set_prgname(prgname: *const std::ffi::c_char);
    }
    let name = std::ffi::CString::new("arcanaglyph").expect("static name without NULs");
    // SAFETY: glib `g_set_prgname` копирует строку в свой буфер; срок жизни
    // нашего CString не важен. Вызывается до спавна потоков и до GTK init.
    unsafe { g_set_prgname(name.as_ptr()) };
}

#[cfg(not(target_os = "linux"))]
pub fn setup_program_name() {}

/// CLI: `arcanaglyph --grant-portal` — запускает только XDG RemoteDesktop
/// warmup и выходит. Используется install.sh после установки .deb/.AppImage:
/// popup от GNOME Shell всплывает в момент инсталляции (где пользователь
/// ожидает диалогов), а не при первом Ctrl+Ё. На X11 / non-Linux — noop.
pub fn run_grant_portal_and_exit() -> ! {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Не удалось запустить async runtime: {e}");
            std::process::exit(2);
        }
    };
    match rt.block_on(arcanaglyph_core::input::warmup_remote_desktop()) {
        Ok(()) => {
            println!("XDG RemoteDesktop permission получен.");
            println!("При следующем запуске приложения popup больше не появится.");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Не удалось получить XDG RemoteDesktop permission: {e}");
            eprintln!("Это не блокирует работу — popup появится при первом Ctrl+Ё.");
            std::process::exit(1);
        }
    }
}
