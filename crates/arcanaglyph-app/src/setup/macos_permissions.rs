// crates/arcanaglyph-app/src/setup/macos_permissions.rs
//
// Логирование статуса macOS-разрешений при старте. macOS гейтит ключевые
// возможности приложения тремя отдельными грантами в System Settings →
// Privacy & Security:
//   * Accessibility    — нужен для симуляции вставки текста (enigo) в чужие окна;
//   * Input Monitoring — нужен для глобального хоткея (перехват нажатий клавиш);
//   * Microphone       — нужен для записи; системный промпт всплывает сам при
//                        первом доступе через cpal (описание — в Info.plist,
//                        ключ NSMicrophoneUsageDescription). Статус здесь не
//                        опрашиваем: нет чистого C-API без AVFoundation/objc.
//
// Цикл итераций на macOS медленный (своего Mac нет, тест через друга в другом
// городе), поэтому статус каждого гранта пишем в лог при старте — denied видно
// сразу из лог-файла, не гадаем по симптомам. Опрос — через прямой FFI к
// системным фреймворкам, без новых крейтов и без objc-messaging.

// FFI к системным фреймворкам — линкуется только на macOS.
#[cfg(target_os = "macos")]
mod ffi {
    // AXIsProcessTrusted (ApplicationServices): возвращает Boolean (unsigned char),
    // != 0 если процессу выдан грант Accessibility.
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        pub fn AXIsProcessTrusted() -> u8;
    }

    // IOHIDCheckAccess (IOKit): статус доступа к HID-событиям (Input Monitoring).
    // request: kIOHIDRequestTypeListenEvent = 1.
    // возврат IOHIDAccessType: 0 = granted, 1 = denied, 2 = unknown.
    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        pub fn IOHIDCheckAccess(request: u32) -> u32;
    }
}

// kIOHIDRequestTypeListenEvent — запрашиваем право слушать (а не постить) события.
#[cfg(target_os = "macos")]
const K_IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;

// Человекочитаемая метка для IOHIDAccessType. Чистая функция (без FFI) —
// компилируется и тестируется на всех платформах, не только в macOS-CI; на
// не-macOS её зовёт только тест, поэтому глушим dead_code там.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn hid_access_label(status: u32) -> &'static str {
    match status {
        0 => "granted",
        1 => "denied — глобальный хоткей не сработает",
        _ => "unknown (ещё не запрашивалось)",
    }
}

/// Логирует статус macOS-разрешений при старте приложения. Вызывается один раз
/// из `run_setup` под `#[cfg(target_os = "macos")]`.
#[cfg(target_os = "macos")]
pub fn log_macos_permission_status() {
    // SAFETY: обе функции — системные C-API фреймворков без аргументов-указателей
    // и без побочных эффектов кроме чтения статуса гранта. Линкуются через
    // `#[link(... kind = "framework")]` в модуле `ffi`.
    let accessibility = unsafe { ffi::AXIsProcessTrusted() } != 0;
    let input_monitoring = unsafe { ffi::IOHIDCheckAccess(K_IOHID_REQUEST_TYPE_LISTEN_EVENT) };

    tracing::info!(
        "[macOS permissions] Accessibility (вставка текста) = {}",
        if accessibility {
            "granted"
        } else {
            "denied — вставка текста не сработает"
        }
    );
    tracing::info!(
        "[macOS permissions] Input Monitoring (глоб. хоткей) = {}",
        hid_access_label(input_monitoring)
    );
    tracing::info!(
        "[macOS permissions] Microphone = системный промпт при первом доступе (cpal); \
         статус не опрашивается (нужен AVFoundation/objc)"
    );
}

#[cfg(test)]
mod tests {
    use super::hid_access_label;

    #[test]
    fn hid_access_label_maps_known_statuses() {
        assert_eq!(hid_access_label(0), "granted");
        assert!(hid_access_label(1).starts_with("denied"));
        assert!(hid_access_label(2).starts_with("unknown"));
        // любой неизвестный код → ветка unknown
        assert!(hid_access_label(99).starts_with("unknown"));
    }
}
