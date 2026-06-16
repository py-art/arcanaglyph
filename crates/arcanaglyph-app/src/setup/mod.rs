// crates/arcanaglyph-app/src/setup/mod.rs
//
// Bootstrap (env, ORT, autostart), тело `.setup(...)` Tauri-приложения
// (engine spawn, виджет, хоткеи, авторегистрация в GNOME), фоновые spawn-задачи
// (engine→frontend event loop, update checker, IPC trigger listener).

pub mod app_setup;
pub mod bootstrap;
pub mod events;
// Логирование статуса macOS-разрешений (Accessibility / Input Monitoring /
// Microphone) при старте. Чистый helper компилируется везде, FFI — только macOS.
pub mod macos_permissions;

pub use app_setup::run_setup;
pub use bootstrap::{run_grant_portal_and_exit, setup_ort_dylib_path, setup_program_name};
