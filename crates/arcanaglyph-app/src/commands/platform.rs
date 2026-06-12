// crates/arcanaglyph-app/src/commands/platform.rs
//
// Команды, которые сообщают фронтенду свойства системы: какой движок входит
// в сборку, какие SIMD-фичи у CPU, Wayland/X11/GNOME окружение, версия
// приложения, имя активного микрофона и т.п. Сюда же кладём команды XDG
// RemoteDesktop warmup — они не привязаны к engine.

use crate::updater;
use arcanaglyph_core::config::TranscriberType;

/// Tauri-команда: имя текущего активного default-микрофона (через cpal).
/// Фронтенд использует для отображения "Активный микрофон: ..." в Settings и
/// чтобы записать gain под правильный device-key в `mic_gain_per_device`.
/// Возвращает пустую строку если устройства нет.
#[tauri::command]
pub fn get_default_input_device_name() -> String {
    arcanaglyph_core::audio::default_input_device_name().unwrap_or_default()
}

/// Tauri-команда: список движков, включённых в текущую сборку (по cargo features).
/// Фронтенд использует это, чтобы пометить недоступные пункты в dropdown'е как disabled.
#[tauri::command]
pub fn get_compiled_engines() -> Vec<&'static str> {
    TranscriberType::compiled_engines()
        .into_iter()
        .map(|t| t.as_str())
        .collect()
}

/// Tauri-команда: какие SIMD-фичи доступны на текущем CPU. Фронтенд использует это,
/// чтобы предупредить пользователя при выборе тяжёлой модели (Whisper Large без AVX2
/// — это 10-30× замедление).
#[tauri::command]
pub fn get_cpu_features() -> serde_json::Value {
    #[cfg(target_arch = "x86_64")]
    {
        serde_json::json!({
            "avx": std::is_x86_feature_detected!("avx"),
            "avx2": std::is_x86_feature_detected!("avx2"),
            "fma": std::is_x86_feature_detected!("fma"),
        })
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        serde_json::json!({
            "avx": false,
            "avx2": false,
            "fma": false,
        })
    }
}

/// Tauri-команда: определить, работает ли Wayland.
/// Делегирует в core (`arcanaglyph_core::input::is_wayland`) — единый источник истины,
/// чтобы детект сессии не расходился между app и core.
#[tauri::command]
pub fn is_wayland() -> bool {
    #[cfg(target_os = "linux")]
    {
        arcanaglyph_core::input::is_wayland()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Tauri-команда: запущены ли мы в GNOME-сессии (любой DM).
/// Используется UI чтобы показывать toggle расширения только там, где оно
/// применимо (на KDE/sway/Cinnamon наше расширение не работает).
#[tauri::command]
pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_uppercase().contains("GNOME") || v.to_uppercase().contains("UNITY"))
        .unwrap_or(false)
}

/// Tauri-команда: вероятно ли всплывёт XDG popup при первом нажатии Ctrl+Ё.
/// UI использует это чтобы показать однократный баннер «Дать разрешение»
/// до первого срабатывания горячей клавиши.
#[tauri::command]
pub fn check_portal_grant_needed() -> bool {
    arcanaglyph_core::input::needs_portal_grant()
}

/// Tauri-команда: запросить XDG RemoteDesktop permission прямо сейчас
/// (eager warmup). UI вызывает это по клику кнопки в баннере;
/// popup всплывает в момент клика — пользователь даёт разрешение в
/// expected моменте, а не при первом Ctrl+Ё.
#[tauri::command]
pub async fn grant_portal_now() -> Result<(), String> {
    arcanaglyph_core::input::warmup_remote_desktop()
        .await
        .map_err(|e| e.to_string())
}

/// Tauri-команда: текущая версия приложения (читается из CARGO_PKG_VERSION
/// через updater::APP_VERSION). UI использует её в About-секции вместо
/// захардкоженной строки — после bump'а версии HTML не нужно править.
#[tauri::command]
pub fn get_app_version() -> &'static str {
    updater::APP_VERSION
}
