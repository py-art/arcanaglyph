// crates/arcanaglyph-app/src/updater/asset.rs
//
// Отбор устанавливаемого asset'а релиза под текущую платформу
// (.deb на Linux, .exe на Windows) и его загрузка.

use arcanaglyph_core::error::ArcanaError;

use super::{CHECK_TIMEOUT_SECS, RELEASES_API, with_github_cache_bust};

/// Расширение устанавливаемого asset'а для текущей платформы: `.deb` на Linux,
/// `.exe` (NSIS-инсталлятор) на Windows. Updater показывает обновление только
/// когда в релизе лежит asset под ИМЕННО эту платформу.
#[cfg(target_os = "windows")]
const INSTALLABLE_ASSET_EXT: &str = ".exe";
#[cfg(not(target_os = "windows"))]
const INSTALLABLE_ASSET_EXT: &str = ".deb";

/// JSON-ключи GitHub Release asset + значение готового к установке состояния.
const ASSET_KEY_NAME: &str = "name";
const ASSET_KEY_STATE: &str = "state";
const ASSET_STATE_UPLOADED: &str = "uploaded";

/// Проверяет, есть ли в JSON релиза готовый к установке asset под текущую
/// платформу (`INSTALLABLE_ASSET_EXT`). «Готовый» = `name` оканчивается на
/// нужное расширение И `state == "uploaded"` (GitHub помечает asset'ы в процессе
/// заливки как `starting`/`uploading`, и только по завершении — `uploaded`).
/// Именно это закрывает гонку «релиз опубликован, но CI ещё собирает/заливает
/// пакеты»: до завершения заливки in-app updater запустил бы установку, которая
/// упала бы на `No assets found in release`.
pub fn release_has_installable_asset(release: &serde_json::Value) -> bool {
    release.get("assets").and_then(|v| v.as_array()).is_some_and(|assets| {
        assets.iter().any(|a| {
            let is_match = a
                .get(ASSET_KEY_NAME)
                .and_then(|n| n.as_str())
                .is_some_and(|n| n.ends_with(INSTALLABLE_ASSET_EXT));
            let uploaded = a
                .get(ASSET_KEY_STATE)
                .and_then(|s| s.as_str())
                .is_some_and(|s| s == ASSET_STATE_UPLOADED);
            is_match && uploaded
        })
    })
}

/// Возвращает `browser_download_url` готового к установке asset'а под текущую
/// платформу (`INSTALLABLE_ASSET_EXT`, `state == "uploaded"`) из JSON релиза,
/// или None. Используется Windows-веткой apply_update для авто-скачивания
/// установщика. Условие отбора то же, что в `release_has_installable_asset`.
pub fn installable_asset_url(release: &serde_json::Value) -> Option<String> {
    release.get("assets").and_then(|v| v.as_array()).and_then(|assets| {
        assets.iter().find_map(|a| {
            let name = a.get(ASSET_KEY_NAME).and_then(|n| n.as_str())?;
            let state = a.get(ASSET_KEY_STATE).and_then(|s| s.as_str())?;
            if name.ends_with(INSTALLABLE_ASSET_EXT) && state == ASSET_STATE_UPLOADED {
                a.get("browser_download_url")
                    .and_then(|u| u.as_str())
                    .map(str::to_string)
            } else {
                None
            }
        })
    })
}

/// Скачивает JSON последнего релиза и возвращает URL установочного asset'а под
/// текущую платформу. В отличие от `check_for_update`, НЕ использует ETag-кэш —
/// на нажатие «Обновить» нужен свежий релиз именно сейчас. Сетевая функция,
/// в unit-тестах не покрывается (логика отбора покрыта тестом на
/// `installable_asset_url`). Компилируется на всех платформах (чтобы Linux-CI
/// ловил ошибки), вызывается только на Windows.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub async fn fetch_installable_asset_url() -> Result<Option<String>, ArcanaError> {
    let client = crate::net::build_http_client(CHECK_TIMEOUT_SECS)?;

    let text = with_github_cache_bust(client.get(RELEASES_API))
        .send()
        .await
        .map_err(|e| ArcanaError::Internal(format!("release fetch: {}", e)))?
        .error_for_status()
        .map_err(|e| ArcanaError::Internal(format!("GitHub API status: {}", e)))?
        .text()
        .await
        .map_err(|e| ArcanaError::Internal(format!("GitHub API read: {}", e)))?;

    let body: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ArcanaError::Internal(format!("GitHub API parse: {}", e)))?;
    Ok(installable_asset_url(&body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deb_available_when_uploaded() {
        let release = serde_json::json!({
            "tag_name": "v1.7.8",
            "assets": [
                { "name": "ArcanaGlyph_1.7.8_amd64.deb", "state": "uploaded" },
                { "name": "ArcanaGlyph_1.7.8_amd64.AppImage", "state": "uploaded" },
                { "name": "SHA256SUMS.txt", "state": "uploaded" }
            ]
        });
        assert!(release_has_installable_asset(&release));
    }

    #[test]
    fn asset_url_returns_uploaded_installer_for_platform() {
        let release = serde_json::json!({
            "assets": [
                { "name": "ArcanaGlyph_1.7.8_amd64.deb", "state": "uploaded",
                  "browser_download_url": "https://example.com/app.deb" },
                { "name": "ArcanaGlyph_1.7.8_x64-setup.exe", "state": "uploaded",
                  "browser_download_url": "https://example.com/app.exe" }
            ]
        });
        let url = installable_asset_url(&release).expect("есть asset под платформу");
        // INSTALLABLE_ASSET_EXT платформо-зависим: .deb на Linux, .exe на Windows.
        if INSTALLABLE_ASSET_EXT == ".deb" {
            assert_eq!(url, "https://example.com/app.deb");
        } else {
            assert_eq!(url, "https://example.com/app.exe");
        }
    }

    #[test]
    fn asset_url_none_when_not_uploaded() {
        let release = serde_json::json!({
            "assets": [
                { "name": format!("ArcanaGlyph_1.7.8_amd64{INSTALLABLE_ASSET_EXT}"),
                  "state": "starting", "browser_download_url": "https://example.com/app" }
            ]
        });
        assert!(installable_asset_url(&release).is_none());
    }

    #[test]
    fn asset_url_none_when_no_matching_ext() {
        // Только asset чужой платформы — под текущую URL'а нет.
        let foreign = if INSTALLABLE_ASSET_EXT == ".deb" {
            ".exe"
        } else {
            ".deb"
        };
        let name = format!("ArcanaGlyph_1.7.8_pkg{foreign}");
        let release = serde_json::json!({
            "assets": [
                { "name": name, "state": "uploaded", "browser_download_url": "https://example.com/other" }
            ]
        });
        assert!(installable_asset_url(&release).is_none());
    }

    #[test]
    fn deb_unavailable_when_assets_empty() {
        // Воспроизводит гонку: релиз опубликован, CI ещё собирает .deb.
        let release = serde_json::json!({ "tag_name": "v1.7.8", "assets": [] });
        assert!(!release_has_installable_asset(&release));
    }

    #[test]
    fn deb_unavailable_when_still_uploading() {
        // .deb уже в списке, но заливка не завершена (state != uploaded).
        let release = serde_json::json!({
            "tag_name": "v1.7.8",
            "assets": [
                { "name": "ArcanaGlyph_1.7.8_amd64.deb", "state": "starting" }
            ]
        });
        assert!(!release_has_installable_asset(&release));
    }

    #[test]
    fn deb_unavailable_when_only_appimage() {
        let release = serde_json::json!({
            "tag_name": "v1.7.8",
            "assets": [
                { "name": "ArcanaGlyph_1.7.8_amd64.AppImage", "state": "uploaded" },
                { "name": "SHA256SUMS.txt", "state": "uploaded" }
            ]
        });
        assert!(!release_has_installable_asset(&release));
    }

    #[test]
    fn deb_unavailable_when_no_assets_field() {
        let release = serde_json::json!({ "tag_name": "v1.7.8" });
        assert!(!release_has_installable_asset(&release));
    }
}
