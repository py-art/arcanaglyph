// crates/arcanaglyph-app/src/net.rs
//
// Единая фабрика HTTP-клиентов для апдейтера + определение системного
// прокси. Мотив: GUI-приложение, запущенное из меню/автозапуска, НЕ
// наследует proxy-переменные окружения из шелла пользователя (~/.zshrc и
// т.п.), а reqwest из коробки читает прокси ТОЛЬКО из env. За фаерволом
// (например, Fastly-хосты raw/objects.githubusercontent.com режутся
// напрямую) это ломало скачивание install.sh/.exe, хотя сама проверка
// обновлений (api.github.com — не Fastly) продолжала работать. Решение —
// прочитать прокси из системных настроек ОС и отдать его reqwest явно.

use std::time::Duration;

use arcanaglyph_core::error::ArcanaError;

/// Собирает reqwest-клиент с общим User-Agent, заданным таймаутом и —
/// если найден — системным прокси. Приоритет: переменные окружения (их
/// reqwest подхватывает сам, уважаем явный выбор пользователя) → системные
/// настройки ОС. Единая точка создания клиента для всех сетевых вызовов
/// апдейтера: убирает дублирование builder'а и гарантирует одинаковое
/// поведение проверки и скачивания.
pub fn build_http_client(timeout_secs: u64) -> Result<reqwest::Client, ArcanaError> {
    let mut builder = reqwest::Client::builder()
        .user_agent(format!("arcanaglyph/{}", crate::updater::APP_VERSION))
        .timeout(Duration::from_secs(timeout_secs));

    // env-прокси reqwest найдёт сам. Системный добавляем ЯВНО только когда
    // env пуст — иначе дублировали бы/конфликтовали с env-автодетектом.
    if env_proxy_present() {
        tracing::debug!("HTTP-клиент: использую proxy из переменных окружения");
    } else if let Some(url) = system_proxy_url() {
        match reqwest::Proxy::all(&url) {
            Ok(proxy) => {
                tracing::info!("HTTP-клиент: использую системный прокси {url}");
                builder = builder.proxy(proxy);
            }
            Err(e) => {
                tracing::warn!("Системный прокси {url} невалиден ({e}) — иду напрямую");
            }
        }
    } else {
        tracing::debug!("HTTP-клиент: прокси не задан, прямое соединение");
    }

    builder
        .build()
        .map_err(|e| ArcanaError::Internal(format!("reqwest client: {e}")))
}

/// Есть ли в окружении хоть одна непустая proxy-переменная, которую reqwest
/// читает сам. Имена — те же, что проверяет reqwest (верхний и нижний регистр).
fn env_proxy_present() -> bool {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .iter()
    .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}

// ── Системный прокси: платформо-зависимое чтение «сырых» значений ──────────
//
// Тонкие обёртки вокруг ОС-механизмов (gsettings / реестр / scutil) НЕ
// покрываются unit-тестами — они дёргают внешнюю среду. Вся разборная
// логика вынесена в чистые `parse_*`-функции ниже, которые тестируются на
// любой платформе (Linux-CI проверяет в т.ч. Windows/macOS-парсеры).

/// Linux/GNOME: читает `org.gnome.system.proxy` через `gsettings`. KDE и
/// прочие DE здесь не покрыты — там полагаемся на env-прокси.
#[cfg(target_os = "linux")]
fn system_proxy_url() -> Option<String> {
    let mode = gsettings_get("org.gnome.system.proxy", "mode")?;
    let https_host = gsettings_get("org.gnome.system.proxy.https", "host").unwrap_or_default();
    let https_port = gsettings_get("org.gnome.system.proxy.https", "port").unwrap_or_default();
    let http_host = gsettings_get("org.gnome.system.proxy.http", "host").unwrap_or_default();
    let http_port = gsettings_get("org.gnome.system.proxy.http", "port").unwrap_or_default();
    parse_gnome_proxy(&mode, &https_host, &https_port, &http_host, &http_port)
}

#[cfg(target_os = "linux")]
fn gsettings_get(schema: &str, key: &str) -> Option<String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Windows: читает прокси из реестра WinINET
/// (`HKCU\...\Internet Settings`, `ProxyEnable` + `ProxyServer`).
#[cfg(target_os = "windows")]
fn system_proxy_url() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let settings = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    let enable: u32 = settings.get_value("ProxyEnable").unwrap_or(0);
    let server: String = settings.get_value("ProxyServer").unwrap_or_default();
    parse_windows_proxy(enable, &server)
}

/// macOS: читает прокси из системной конфигурации через `scutil --proxy`.
#[cfg(target_os = "macos")]
fn system_proxy_url() -> Option<String> {
    let out = std::process::Command::new("scutil").arg("--proxy").output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_macos_scutil(&String::from_utf8_lossy(&out.stdout))
}

// ── Чистые парсеры (тестируемы на любой платформе) ─────────────────────────

/// Строит proxy-URL из значений gsettings `org.gnome.system.proxy`. Значения
/// приходят как из `gsettings get`: строки в одинарных кавычках, число без
/// кавычек. Прокси даёт только режим `manual`; `none`/`auto` (PAC) → None
/// (PAC пока не поддерживаем). HTTPS приоритетнее HTTP — наш трафик TLS.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_gnome_proxy(
    mode: &str,
    https_host: &str,
    https_port: &str,
    http_host: &str,
    http_port: &str,
) -> Option<String> {
    if unquote(mode) != "manual" {
        return None;
    }
    proxy_url_from(unquote(https_host), https_port).or_else(|| proxy_url_from(unquote(http_host), http_port))
}

/// Разбирает значение реестра `ProxyServer`. Формат: либо общий `host:port`,
/// либо per-scheme `http=h:p;https=h:p;...`. Активен только при
/// `ProxyEnable == 1`. Возвращаем HTTPS-прокси (или общий) как `http://…`:
/// WinINET-прокси — это HTTP-прокси с CONNECT-туннелем для TLS.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_windows_proxy(enable: u32, server: &str) -> Option<String> {
    if enable != 1 {
        return None;
    }
    let server = server.trim();
    if server.is_empty() {
        return None;
    }
    if server.contains('=') {
        // per-scheme: предпочитаем https=, иначе http=.
        let find = |scheme: &str| -> Option<String> {
            server
                .split(';')
                .find_map(|part| part.trim().strip_prefix(scheme).map(|hp| hp.trim().to_string()))
        };
        let hp = find("https=").or_else(|| find("http="))?;
        prefix_http(&hp)
    } else {
        prefix_http(server)
    }
}

/// Разбирает вывод `scutil --proxy`. Берём HTTPS-прокси, если включён
/// (`HTTPSEnable : 1`), формируя `http://HTTPSProxy:HTTPSPort`. Строки
/// вида `  KEY : VALUE`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_macos_scutil(output: &str) -> Option<String> {
    let mut map = std::collections::HashMap::new();
    for line in output.lines() {
        if let Some((k, v)) = line.split_once(':') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    if map.get("HTTPSEnable").map(String::as_str) != Some("1") {
        return None;
    }
    proxy_url_from(map.get("HTTPSProxy")?, map.get("HTTPSPort")?)
}

/// `host` + `port` → `http://host:port`. None, если host/port пусты или порт 0.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn proxy_url_from(host: &str, port: &str) -> Option<String> {
    let host = host.trim();
    let port = port.trim();
    if host.is_empty() || port.is_empty() || port == "0" {
        return None;
    }
    Some(format!("http://{host}:{port}"))
}

/// Добавляет схему `http://` к `host:port`, если её нет (Windows-прокси).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn prefix_http(host_port: &str) -> Option<String> {
    let hp = host_port.trim();
    if hp.is_empty() {
        return None;
    }
    if hp.starts_with("http://") || hp.starts_with("https://") {
        Some(hp.to_string())
    } else {
        Some(format!("http://{hp}"))
    }
}

/// Снимает обрамляющие одинарные кавычки и пробелы со значения gsettings.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn unquote(s: &str) -> &str {
    s.trim().trim_matches('\'').trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_client_always_succeeds() {
        // Фабрика обязана всегда вернуть клиент: невалидный/недоступный
        // системный прокси логируется, но НЕ роняет билд (иначе апдейтер
        // умер бы вместо обхода через прямое соединение).
        assert!(build_http_client(10).is_ok());
        assert!(build_http_client(600).is_ok());
    }

    #[test]
    fn gnome_manual_prefers_https() {
        let url = parse_gnome_proxy("'manual'", "'127.0.0.1'", "2080", "'10.0.0.1'", "3128");
        assert_eq!(url.as_deref(), Some("http://127.0.0.1:2080"));
    }

    #[test]
    fn gnome_manual_falls_back_to_http() {
        let url = parse_gnome_proxy("'manual'", "''", "0", "'10.0.0.1'", "3128");
        assert_eq!(url.as_deref(), Some("http://10.0.0.1:3128"));
    }

    #[test]
    fn gnome_none_and_auto_are_ignored() {
        assert_eq!(parse_gnome_proxy("'none'", "'127.0.0.1'", "2080", "", ""), None);
        assert_eq!(parse_gnome_proxy("'auto'", "'127.0.0.1'", "2080", "", ""), None);
    }

    #[test]
    fn gnome_manual_but_empty_is_none() {
        assert_eq!(parse_gnome_proxy("'manual'", "''", "0", "''", "0"), None);
    }

    #[test]
    fn windows_disabled_is_none() {
        assert_eq!(parse_windows_proxy(0, "127.0.0.1:2080"), None);
    }

    #[test]
    fn windows_simple_host_port() {
        assert_eq!(
            parse_windows_proxy(1, "127.0.0.1:2080").as_deref(),
            Some("http://127.0.0.1:2080")
        );
    }

    #[test]
    fn windows_per_scheme_prefers_https() {
        let url = parse_windows_proxy(1, "http=10.0.0.1:3128;https=127.0.0.1:2080");
        assert_eq!(url.as_deref(), Some("http://127.0.0.1:2080"));
    }

    #[test]
    fn windows_per_scheme_http_only() {
        let url = parse_windows_proxy(1, "ftp=1.1.1.1:21;http=10.0.0.1:3128");
        assert_eq!(url.as_deref(), Some("http://10.0.0.1:3128"));
    }

    #[test]
    fn windows_empty_is_none() {
        assert_eq!(parse_windows_proxy(1, "   "), None);
    }

    #[test]
    fn macos_https_enabled() {
        let out = "<dictionary> {\n  HTTPSEnable : 1\n  HTTPSProxy : 127.0.0.1\n  HTTPSPort : 2080\n}";
        assert_eq!(parse_macos_scutil(out).as_deref(), Some("http://127.0.0.1:2080"));
    }

    #[test]
    fn macos_https_disabled_is_none() {
        let out = "<dictionary> {\n  HTTPSEnable : 0\n  HTTPSProxy : 127.0.0.1\n  HTTPSPort : 2080\n}";
        assert_eq!(parse_macos_scutil(out), None);
    }

    #[test]
    fn macos_missing_keys_is_none() {
        assert_eq!(parse_macos_scutil("<dictionary> {\n}"), None);
    }
}
