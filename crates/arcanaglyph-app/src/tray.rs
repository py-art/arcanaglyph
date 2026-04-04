// crates/arcanaglyph-app/src/tray.rs

use arcanaglyph_core::ArcanaEngine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{
    AppHandle, Manager,
    menu::{Menu, MenuEvent, MenuItem},
    tray::TrayIconBuilder,
};

/// Обёртка для хранения toggle-пункта меню в Tauri state
pub struct TrayToggleItem(pub MenuItem<tauri::Wry>);

/// Показать окно и поставить в фокус
pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    if let Some(vis) = app.try_state::<Arc<AtomicBool>>() {
        vis.store(true, Ordering::Relaxed);
    }
}

/// Создаёт иконку в системном трее с меню управления
pub fn create_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show_item = MenuItem::with_id(app, "show", "Открыть приложение", true, None::<&str>)?;
    let toggle_item = MenuItem::with_id(app, "toggle", "Начать запись", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &toggle_item, &quit_item])?;

    // Сохраняем toggle_item в state для обновления текста при смене состояния
    app.manage(TrayToggleItem(toggle_item));

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("ArcanaGlyph")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event: MenuEvent| match event.id().as_ref() {
            "show" => {
                show_window(app);
            }
            "toggle" => {
                if let Some(engine) = app.try_state::<Arc<ArcanaEngine>>() {
                    engine.trigger();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    Ok(())
}

/// Обновляет текст toggle-пункта в меню трея
pub fn set_tray_text(app: &AppHandle, text: &str) {
    if let Some(item) = app.try_state::<TrayToggleItem>() {
        let _ = item.0.set_text(text);
    }
}
