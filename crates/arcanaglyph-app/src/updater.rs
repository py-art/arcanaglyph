// crates/arcanaglyph-app/src/updater.rs
//
// Авто-обновления ArcanaGlyph: фоновая проверка GitHub Releases,
// баннер «Доступно обновление» в UI, запуск install.sh в spawned-терминале.
//
// Persistence — через существующий HistoryDB::set_setting/get_setting
// (ключ "update_state", значение — JSON UpdateState). То же самое
// что делает CoreConfig — никаких новых файлов в data_dir.
//
// Версия читается через env!(CARGO_PKG_VERSION) этого крейта (app),
// а не core: именно версия arcanaglyph-app фигурирует в .deb / AppImage
// (tauri.conf.json синхронизируется с app/Cargo.toml). У core своя
// версия, и она может разойтись.

use std::time::{SystemTime, UNIX_EPOCH};

use arcanaglyph_core::error::ArcanaError;
use arcanaglyph_core::history::HistoryDB;
use serde::{Deserialize, Serialize};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
// На Windows/macOS in-app установка через install.sh не используется (apply_update
// открывает страницу релиза), поэтому константа там «мёртвая» — глушим warning.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const INSTALL_URL: &str = "https://github.com/py-art/arcanaglyph/raw/main/install.sh";

const STATE_KEY: &str = "update_state";
const RELEASES_API: &str = "https://api.github.com/repos/py-art/arcanaglyph/releases/latest";
const CHECK_TIMEOUT_SECS: u64 = 10;

/// Информация о доступном обновлении, отдаётся фронту через `app.emit`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub latest_version: String,
    pub release_url: String,
    pub published_at: String,
}

/// Результат запроса к GitHub Releases API. Разделяет три состояния,
/// чтобы `check_for_update` мог корректно вести ETag-кэш и при этом НЕ
/// показывать баннер, пока обновление реально нельзя установить.
#[derive(Debug, Clone)]
pub enum ReleaseFetch {
    /// 304 Not Modified — состояние релиза не изменилось с прошлого ETag.
    NotModified,
    /// Релиз получен И в нём есть готовый к установке `.deb` asset.
    Available { info: UpdateInfo, etag: Option<String> },
    /// Релиз получен, но устанавливать нечего: либо `.deb` ещё не залит
    /// (CI всё ещё собирает пакеты), либо tag нестандартный/pre-release.
    /// ETag всё равно возвращаем — caller его сохранит, чтобы следующая
    /// проверка получила 304/200 корректно (заливка asset'а меняет
    /// представление релиза → ETag сменится → 200 с готовым `.deb`).
    Unavailable { etag: Option<String> },
}

/// Расширение устанавливаемого asset'а для текущей платформы: `.deb` на Linux,
/// `.exe` (NSIS-инсталлятор) на Windows. Updater показывает обновление только
/// когда в релизе лежит asset под ИМЕННО эту платформу.
#[cfg(target_os = "windows")]
const INSTALLABLE_ASSET_EXT: &str = ".exe";
#[cfg(not(target_os = "windows"))]
const INSTALLABLE_ASSET_EXT: &str = ".deb";

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
                .get("name")
                .and_then(|n| n.as_str())
                .is_some_and(|n| n.ends_with(INSTALLABLE_ASSET_EXT));
            let uploaded = a.get("state").and_then(|s| s.as_str()).is_some_and(|s| s == "uploaded");
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
            let name = a.get("name").and_then(|n| n.as_str())?;
            let state = a.get("state").and_then(|s| s.as_str())?;
            if name.ends_with(INSTALLABLE_ASSET_EXT) && state == "uploaded" {
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
    let client = reqwest::Client::builder()
        .user_agent(format!("arcanaglyph/{}", APP_VERSION))
        .timeout(std::time::Duration::from_secs(CHECK_TIMEOUT_SECS))
        .build()
        .map_err(|e| ArcanaError::Internal(format!("reqwest client: {}", e)))?;

    let text = client
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
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

/// Состояние update-checker'а. Хранится в SQLite через `set_setting`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct UpdateState {
    /// Unix timestamp последней удачной (или 304) проверки.
    pub last_check_at: Option<i64>,
    /// Самая свежая версия которую видел checker (без 'v'-префикса).
    pub latest_known: Option<String>,
    pub latest_release_url: Option<String>,
    pub latest_published_at: Option<String>,
    /// Версия которую пользователь dismiss'нул крестиком. Если выйдет
    /// ещё более новая — баннер вернётся.
    pub dismissed_version: Option<String>,
    /// Версия, для которой пользователь нажал «Обновить». Пока поле
    /// заполнено и не совпадает с `APP_VERSION`, UI показывает баннер в
    /// applying-режиме (прогресс + кнопка «Перезапустить»). Очищается
    /// при старте, когда `APP_VERSION` догнал значение поля.
    pub applying_version: Option<String>,
    /// ETag от GitHub Releases — позволяет получать 304 Not Modified
    /// и не сжигать rate-limit на одинаковых запросах.
    pub etag: Option<String>,
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn read_state(db: &HistoryDB) -> UpdateState {
    db.get_setting(STATE_KEY)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn write_state(db: &HistoryDB, state: &UpdateState) -> Result<(), ArcanaError> {
    let json =
        serde_json::to_string(state).map_err(|e| ArcanaError::Internal(format!("update_state serialize: {}", e)))?;
    db.set_setting(STATE_KEY, &json)
}

/// Парсер git tag в `(major, minor, patch)`. Pre-release / build suffixes
/// (`-rc1`, `+build.5`) возвращают None — мы не показываем пользователю
/// нестабильные релизы.
pub fn parse_release_tag(tag: &str) -> Option<(u32, u32, u32)> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    if stripped.contains('-') || stripped.contains('+') {
        return None;
    }
    let parts: Vec<&str> = stripped.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_release_tag(latest), parse_release_tag(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// Запрашивает GitHub Releases API. Возвращает:
/// - `Ok(ReleaseFetch::Available { info, etag })` — релиз есть И в нём
///   лежит готовый к установке `.deb`.
/// - `Ok(ReleaseFetch::Unavailable { etag })` — релиз есть, но `.deb` ещё
///   не залит (CI собирает) ЛИБО tag pre-release / нестандартный.
/// - `Ok(ReleaseFetch::NotModified)` — 304 Not Modified.
/// - `Err(_)` — сетевая/HTTP ошибка. Caller применяет exponential backoff.
pub async fn fetch_latest_release(etag: Option<&str>) -> Result<ReleaseFetch, ArcanaError> {
    // User-Agent обязателен: без него GitHub возвращает 403 + сообщение
    // "Request forbidden by administrative rules".
    let client = reqwest::Client::builder()
        .user_agent(format!("arcanaglyph/{}", APP_VERSION))
        .timeout(std::time::Duration::from_secs(CHECK_TIMEOUT_SECS))
        .build()
        .map_err(|e| ArcanaError::Internal(format!("reqwest client: {}", e)))?;

    let mut req = client
        .get(RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");

    if let Some(etag) = etag {
        req = req.header("If-None-Match", etag);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| ArcanaError::Internal(format!("update fetch: {}", e)))?;

    let status = resp.status();
    if status.as_u16() == 304 {
        return Ok(ReleaseFetch::NotModified);
    }
    if !status.is_success() {
        return Err(ArcanaError::Internal(format!("GitHub API status {}", status.as_u16())));
    }

    let new_etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // reqwest у нас собран с features = ["stream"] без "json", поэтому
    // парсим вручную через text() + serde_json. Не добавляем "json"
    // фичу — она тянет лишние deps (mime).
    let text = resp
        .text()
        .await
        .map_err(|e| ArcanaError::Internal(format!("GitHub API read: {}", e)))?;
    let body: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| ArcanaError::Internal(format!("GitHub API parse: {}", e)))?;

    let tag_name = body.get("tag_name").and_then(|v| v.as_str()).unwrap_or_default();

    if parse_release_tag(tag_name).is_none() {
        // Нестандартный / pre-release tag — обновлением не считаем, но
        // ETag сохраняем (Unavailable), чтобы следующий запрос был 304.
        return Ok(ReleaseFetch::Unavailable { etag: new_etag });
    }

    let release_url = body
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let published_at = body
        .get("published_at")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let latest_version = tag_name.strip_prefix('v').unwrap_or(tag_name).to_string();

    // Ключевая проверка: показываем обновление ТОЛЬКО когда в релизе уже
    // лежит готовый asset под текущую платформу (.deb/.exe). Иначе релиз
    // опубликован, но CI ещё собирает пакеты (см. `release_has_installable_asset`).
    if !release_has_installable_asset(&body) {
        return Ok(ReleaseFetch::Unavailable { etag: new_etag });
    }

    Ok(ReleaseFetch::Available {
        info: UpdateInfo {
            latest_version,
            release_url,
            published_at,
        },
        etag: new_etag,
    })
}

/// Полный цикл проверки: GitHub fetch → state update → возврат
/// `UpdateInfo` если есть newer и не dismissed. Используется и фоновым
/// checker'ом, и manual-кнопкой «Проверить обновления».
pub async fn check_for_update(db: &HistoryDB) -> Result<Option<UpdateInfo>, ArcanaError> {
    let mut state = read_state(db);
    let fetch = fetch_latest_release(state.etag.as_deref()).await?;

    state.last_check_at = Some(unix_now());

    let info = match fetch {
        ReleaseFetch::Available { info, etag } => {
            state.latest_known = Some(info.latest_version.clone());
            state.latest_release_url = Some(info.release_url.clone());
            state.latest_published_at = Some(info.published_at.clone());
            if let Some(etag) = etag {
                state.etag = Some(etag);
            }
            Some(info)
        }
        ReleaseFetch::Unavailable { etag } => {
            // Релиз есть, но устанавливать пока нечего (.deb не залит /
            // pre-release tag). ETag сохраняем — politeness к rate-limit,
            // но `latest_known` НЕ трогаем, чтобы баннер не появился, пока
            // обновление реально не станет устанавливаемым.
            if let Some(etag) = etag {
                state.etag = Some(etag);
            }
            None
        }
        ReleaseFetch::NotModified => None,
    };

    write_state(db, &state)?;

    Ok(info.filter(|i| {
        is_newer(&i.latest_version, APP_VERSION)
            && state.dismissed_version.as_deref() != Some(i.latest_version.as_str())
            && state.applying_version.as_deref() != Some(i.latest_version.as_str())
    }))
}

/// Достаёт UpdateInfo из state без HTTP-запроса. Используется при старте
/// приложения, чтобы UI получил баннер мгновенно (до того как фоновый
/// чекер сделает свой первый fetch через 60 секунд).
pub fn cached_pending_update(db: &HistoryDB) -> Option<UpdateInfo> {
    let state = read_state(db);
    let latest_version = state.latest_known?;
    if !is_newer(&latest_version, APP_VERSION) {
        return None;
    }
    if state.dismissed_version.as_deref() == Some(latest_version.as_str()) {
        return None;
    }
    if state.applying_version.as_deref() == Some(latest_version.as_str()) {
        return None;
    }
    Some(UpdateInfo {
        latest_version,
        release_url: state.latest_release_url.unwrap_or_default(),
        published_at: state.latest_published_at.unwrap_or_default(),
    })
}

/// Записывает `version` в `dismissed_version`. Баннер не появится для
/// этой версии — но если выйдет ещё более новая, вернётся.
pub fn dismiss(db: &HistoryDB, version: &str) -> Result<(), ArcanaError> {
    let mut state = read_state(db);
    state.dismissed_version = Some(version.to_string());
    write_state(db, &state)
}

/// Помечает версию как «устанавливается». UI переходит в applying-режим
/// (прогресс + «Перезапустить»), баннер «Доступно» не показывается
/// поверх. Сбрасывается при старте, когда `APP_VERSION` догнал значение.
/// Вызывается из Linux- и Windows-веток apply_update — на macOS «мёртвая»
/// (там apply_update только открывает страницу релиза).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn set_applying(db: &HistoryDB, version: &str) -> Result<(), ArcanaError> {
    let mut state = read_state(db);
    state.applying_version = Some(version.to_string());
    write_state(db, &state)
}

/// Стирает applying-метку. Используется когда пользователь закрыл
/// applying-баннер крестиком (передумал перезапускаться).
pub fn clear_applying(db: &HistoryDB) -> Result<(), ArcanaError> {
    let mut state = read_state(db);
    state.applying_version = None;
    write_state(db, &state)
}

/// Возвращает текущее значение applying_version (None если нет).
/// Используется фронтом на mount баннера, чтобы восстановить
/// applying-режим без ожидания emit'а.
pub fn applying_version(db: &HistoryDB) -> Option<String> {
    read_state(db).applying_version
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_tag() {
        assert_eq!(parse_release_tag("v1.7.0"), Some((1, 7, 0)));
        assert_eq!(parse_release_tag("1.7.0"), Some((1, 7, 0)));
        assert_eq!(parse_release_tag("v1.6.10"), Some((1, 6, 10)));
        assert_eq!(parse_release_tag("v0.0.1"), Some((0, 0, 1)));
    }

    #[test]
    fn parse_skip_pre_release() {
        assert_eq!(parse_release_tag("v1.7.0-rc1"), None);
        assert_eq!(parse_release_tag("v1.7.0-beta.1"), None);
    }

    #[test]
    fn parse_skip_build_metadata() {
        assert_eq!(parse_release_tag("v1.7.0+build.5"), None);
    }

    #[test]
    fn parse_invalid() {
        assert_eq!(parse_release_tag(""), None);
        assert_eq!(parse_release_tag("v1.7"), None);
        assert_eq!(parse_release_tag("not-a-version"), None);
        assert_eq!(parse_release_tag("v1.a.b"), None);
    }

    #[test]
    fn newer_basic() {
        assert!(is_newer("1.7.0", "1.6.9"));
        assert!(is_newer("2.0.0", "1.99.99"));
    }

    #[test]
    fn newer_minor_double_digit() {
        // Sanity-check на наивный лексикографический compare.
        assert!(is_newer("1.6.10", "1.6.9"));
    }

    #[test]
    fn newer_same_or_older() {
        assert!(!is_newer("1.6.9", "1.6.9"));
        assert!(!is_newer("1.6.8", "1.6.9"));
    }

    #[test]
    fn newer_invalid_returns_false() {
        assert!(!is_newer("rc-foo", "1.6.9"));
        assert!(!is_newer("1.6.9", "rc-foo"));
        assert!(!is_newer("v1.7.0-rc1", "1.6.9"));
    }

    /// applying_version блокирует available-баннер: пока сидим в
    /// applying-режиме, фоновый чекер не должен «переключать» UI
    /// обратно в available для той же версии.
    #[test]
    fn cached_pending_skips_when_applying() {
        let info = UpdateInfo {
            latest_version: "9.9.9".into(),
            release_url: "https://example.com".into(),
            published_at: "2026-05-10T00:00:00Z".into(),
        };

        let state_idle = UpdateState {
            latest_known: Some(info.latest_version.clone()),
            latest_release_url: Some(info.release_url.clone()),
            latest_published_at: Some(info.published_at.clone()),
            ..Default::default()
        };
        assert!(state_idle.applying_version.is_none(), "baseline: applying_version пуст");

        let state_applying = UpdateState {
            applying_version: Some(info.latest_version.clone()),
            ..state_idle.clone()
        };
        let blocked = state_applying.applying_version.as_deref() == Some(info.latest_version.as_str());
        assert!(blocked, "при applying_version=latest баннер не показываем");
    }

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

    /// Временная HistoryDB под тест (как `temp_db` в core::history).
    fn temp_history_db(name: &str) -> HistoryDB {
        let base = std::env::temp_dir().join(format!("arcanaglyph_updater_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("create temp dir");
        HistoryDB::new(&base.join("history.db"), base.join("audio")).expect("history db")
    }

    #[test]
    fn read_state_defaults_when_absent() {
        let db = temp_history_db("absent");
        let state = read_state(&db);
        assert!(state.last_check_at.is_none());
        assert!(state.latest_known.is_none());
        assert!(state.etag.is_none());
    }

    #[test]
    fn write_then_read_state_roundtrips() {
        let db = temp_history_db("roundtrip");
        let state = UpdateState {
            last_check_at: Some(1_700_000_000),
            latest_known: Some("1.8.0".into()),
            latest_release_url: Some("https://example.com/r".into()),
            latest_published_at: Some("2026-05-10T00:00:00Z".into()),
            dismissed_version: Some("1.7.9".into()),
            applying_version: None,
            etag: Some("W/\"abc\"".into()),
        };
        write_state(&db, &state).expect("write");

        let got = read_state(&db);
        assert_eq!(got.last_check_at, state.last_check_at);
        assert_eq!(got.latest_known, state.latest_known);
        assert_eq!(got.latest_release_url, state.latest_release_url);
        assert_eq!(got.latest_published_at, state.latest_published_at);
        assert_eq!(got.dismissed_version, state.dismissed_version);
        assert_eq!(got.applying_version, state.applying_version);
        assert_eq!(got.etag, state.etag);
    }
}
