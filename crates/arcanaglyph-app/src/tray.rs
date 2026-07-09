// crates/arcanaglyph-app/src/tray.rs

use arcanaglyph_core::ArcanaEngine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tauri::{
    AppHandle, Emitter, Manager,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
};

/// Обёртка для хранения toggle-пункта меню в Tauri state
pub struct TrayToggleItem(pub MenuItem<tauri::Wry>);

/// Обёртка для хранения TrayIcon в Tauri state
pub struct TrayIconHandle(pub TrayIcon);

/// Красная иконка для режима записи (встроена при компиляции)
const RECORDING_ICON: &[u8] = include_bytes!("../icons/32x32-recording.png");
/// Оранжевая иконка для режима паузы
const PAUSED_ICON: &[u8] = include_bytes!("../icons/32x32-paused.png");

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
    let settings_item = MenuItem::with_id(app, "settings", "Настройки", true, None::<&str>)?;
    let toggle_item = MenuItem::with_id(app, "toggle", "Начать запись", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&show_item, &sep1, &toggle_item, &settings_item, &sep2, &quit_item],
    )?;

    // Сохраняем toggle_item в state для обновления текста при смене состояния
    app.manage(TrayToggleItem(toggle_item));

    let tray = TrayIconBuilder::new()
        .icon(
            app.default_window_icon()
                .ok_or("default window icon not configured")?
                .clone(),
        )
        .tooltip("ArcanaGlyph")
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event: MenuEvent| match event.id().as_ref() {
            "show" => {
                show_window(app);
            }
            "settings" => {
                show_window(app);
                let _ = app.emit("tray://open-settings", ());
            }
            "toggle" => {
                if let Some(engine_state) = app.try_state::<Arc<OnceLock<ArcanaEngine>>>()
                    && let Some(engine) = engine_state.get()
                {
                    engine.trigger();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;

    // Сохраняем TrayIcon в state для смены иконки
    app.manage(TrayIconHandle(tray));
    app.manage(TrayCurrentState(std::sync::Mutex::new(TrayState::Idle)));

    Ok(())
}

/// Общий хелпер: выполнить действие над managed-состоянием трея, если оно
/// зарегистрировано (иначе тихо пропустить). Убирает дублирование паттерна
/// `try_state::<T>()` + действие в set_tray_text / set_tray_visible / set_tray_state.
fn with_tray_state<T, F>(app: &AppHandle, f: F)
where
    T: Send + Sync + 'static,
    F: FnOnce(tauri::State<'_, T>),
{
    if let Some(state) = app.try_state::<T>() {
        f(state);
    }
}

/// Обновляет текст toggle-пункта в меню трея
pub fn set_tray_text(app: &AppHandle, text: &str) {
    with_tray_state::<TrayToggleItem, _>(app, |item| {
        let _ = item.0.set_text(text);
    });
}

/// Состояние иконки трея
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    /// Обычное состояние (белая иконка)
    Idle,
    /// Запись (красная иконка)
    Recording,
    /// Пауза (оранжевая иконка)
    Paused,
}

/// Хранение текущего состояния трея для предотвращения лишних обновлений
pub struct TrayCurrentState(pub std::sync::Mutex<TrayState>);

/// Переключает иконку трея в зависимости от состояния.
/// Пропускает обновление если состояние не изменилось (предотвращает мерцание).
pub fn set_tray_state(app: &AppHandle, state: TrayState) {
    // Проверяем, изменилось ли состояние
    if let Some(current) = app.try_state::<TrayCurrentState>() {
        // Кэш состояния трея (anti-flicker) — не критичен: при отравленном
        // мьютексе восстанавливаем guard, а не паникуем в UI-потоке.
        let mut guard = current.0.lock().unwrap_or_else(|e| e.into_inner());
        if *guard == state {
            return;
        }
        *guard = state;
    }

    with_tray_state::<TrayIconHandle, _>(app, |tray| {
        let icon = match state {
            TrayState::Recording => tauri::image::Image::from_bytes(RECORDING_ICON).ok(),
            TrayState::Paused => tauri::image::Image::from_bytes(PAUSED_ICON).ok(),
            TrayState::Idle => app.default_window_icon().cloned(),
        };
        if let Some(icon) = icon {
            let _ = tray.0.set_icon(Some(icon));
        }
    });
}

/// Обратная совместимость: переключает запись/нет
pub fn set_tray_recording(app: &AppHandle, recording: bool) {
    set_tray_state(
        app,
        if recording {
            TrayState::Recording
        } else {
            TrayState::Idle
        },
    );
}

/// Показать или скрыть иконку трея
pub fn set_tray_visible(app: &AppHandle, visible: bool) {
    with_tray_state::<TrayIconHandle, _>(app, |tray| {
        let _ = tray.0.set_visible(visible);
    });
}
