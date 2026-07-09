// crates/arcanaglyph-app/src/updater/mod.rs
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
//
// Модуль разбит на подмодули по концернам (символы реэкспортированы, чтобы
// путь `updater::*` для внешних потребителей не менялся):
//   version.rs — разбор/сравнение версий (parse_release_tag, is_newer)
//   asset.rs   — отбор устанавливаемого asset'а под платформу (.deb/.exe)
//   state.rs   — UpdateState persistence + флаги dismissed/applying
// Здесь остаётся HTTP-оркестрация: fetch_latest_release + check_for_update.

use arcanaglyph_core::error::ArcanaError;
use arcanaglyph_core::history::HistoryDB;
use serde::{Deserialize, Serialize};

mod asset;
mod state;
mod version;

pub use asset::*;
pub use state::*;
pub use version::*;

use self::state::pending_info_from_state;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
// На Windows/macOS in-app установка через install.sh не используется (apply_update
// открывает страницу релиза), поэтому константа там «мёртвая» — глушим warning.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const INSTALL_URL: &str = "https://github.com/py-art/arcanaglyph/raw/main/install.sh";

const RELEASES_API: &str = "https://api.github.com/repos/py-art/arcanaglyph/releases/latest";
const CHECK_TIMEOUT_SECS: u64 = 10;

/// Навешивает на GET-запрос к GitHub Releases API общие заголовки + cache-buster.
/// Cache-buster (уникальный query `_` + `no-cache` заголовки): GitHub-CDN edge и
/// промежуточные прокси иногда залипают на старом снимке релиза (поймали на
/// ноутбуке без AVX: релиз уже с готовым .deb, а API отдавал версию без него →
/// баннер молчал, install.sh падал на «.deb asset not found»). ETag (If-None-Match)
/// продолжает работать: GitHub считает ETag по ресурсу, query `_` игнорит.
/// Дублировался в `fetch_latest_release` и `fetch_installable_asset_url`.
pub(super) fn with_github_cache_bust(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    req.query(&[("_", unix_now().to_string())])
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
}

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

/// Запрашивает GitHub Releases API. Возвращает:
/// - `Ok(ReleaseFetch::Available { info, etag })` — релиз есть И в нём
///   лежит готовый к установке `.deb`.
/// - `Ok(ReleaseFetch::Unavailable { etag })` — релиз есть, но `.deb` ещё
///   не залит (CI собирает) ЛИБО tag pre-release / нестандартный.
/// - `Ok(ReleaseFetch::NotModified)` — 304 Not Modified.
/// - `Err(_)` — сетевая/HTTP ошибка. Caller применяет exponential backoff.
pub async fn fetch_latest_release(etag: Option<&str>) -> Result<ReleaseFetch, ArcanaError> {
    // User-Agent обязателен (его проставляет build_http_client): без него
    // GitHub возвращает 403 + "Request forbidden by administrative rules".
    let client = crate::net::build_http_client(CHECK_TIMEOUT_SECS)?;

    let mut req = with_github_cache_bust(client.get(RELEASES_API));

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
/// checker'ом (`force == false`), и manual-кнопкой «Проверить обновления»
/// (`force == true`).
///
/// `force` пропускает ETag-кэш: ручная проверка всегда запрашивает полный
/// 200 со свежим состоянием релиза вместо 304. Это лечит залипание на старом
/// ETag, когда GitHub однажды отдал снимок без готового `.deb` (CI ещё заливал
/// пакеты / залипший CDN edge) — иначе последующие 304 держали бы баннер
/// молчащим, пока релиз давно установим.
pub async fn check_for_update(db: &HistoryDB, force: bool) -> Result<Option<UpdateInfo>, ArcanaError> {
    let mut state = read_state(db);
    let etag = if force { None } else { state.etag.as_deref() };
    let fetch = fetch_latest_release(etag).await?;

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
        ReleaseFetch::NotModified => {
            // 304: содержимое релиза не изменилось с прошлого fetch. Берём уже
            // зафиксированный `latest_known` из кэша — иначе известное обновление
            // «пропадало» на первом же 304 (баг: «Установлена последняя версия»
            // при наличии newer установимого .deb). Актуальность отсеет общий
            // фильтр is_newer/dismissed/applying ниже.
            pending_info_from_state(&state)
        }
    };

    write_state(db, &state)?;

    Ok(info.filter(|i| {
        is_newer(&i.latest_version, APP_VERSION)
            && state.dismissed_version.as_deref() != Some(i.latest_version.as_str())
            && state.applying_version.as_deref() != Some(i.latest_version.as_str())
    }))
}
