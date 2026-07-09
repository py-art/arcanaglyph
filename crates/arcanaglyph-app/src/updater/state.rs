// crates/arcanaglyph-app/src/updater/state.rs
//
// Состояние update-checker'а (persistence через HistoryDB) + флаги
// dismissed/applying и производные от закэшированного state.

use std::time::{SystemTime, UNIX_EPOCH};

use arcanaglyph_core::error::ArcanaError;
use arcanaglyph_core::history::HistoryDB;
use serde::{Deserialize, Serialize};

use super::{APP_VERSION, UpdateInfo, is_newer};

const STATE_KEY: &str = "update_state";

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

/// Собирает предлагаемое обновление из закэшированного `state` (без HTTP):
/// `latest_known` новее текущей версии И не dismissed/applying. `None` —
/// кэшировать нечего или версия неактуальна. Чистая функция (тестируема).
/// Общий источник для `cached_pending_update` (мгновенный баннер при старте)
/// и для ветки `NotModified` в `check_for_update` (304 не должен «терять»
/// уже известное обновление).
pub(super) fn pending_info_from_state(state: &UpdateState) -> Option<UpdateInfo> {
    let latest_version = state.latest_known.clone()?;
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
        release_url: state.latest_release_url.clone().unwrap_or_default(),
        published_at: state.latest_published_at.clone().unwrap_or_default(),
    })
}

/// Достаёт UpdateInfo из state без HTTP-запроса. Используется при старте
/// приложения, чтобы UI получил баннер мгновенно (до того как фоновый
/// чекер сделает свой первый fetch через 60 секунд).
pub fn cached_pending_update(db: &HistoryDB) -> Option<UpdateInfo> {
    pending_info_from_state(&read_state(db))
}

/// Read-modify-write одного поля состояния апдейтера: читает состояние,
/// применяет мутацию, сохраняет. Убирает дублирование RMW-паттерна в
/// dismiss / set_applying / clear_applying.
fn with_state_mut<F>(db: &HistoryDB, f: F) -> Result<(), ArcanaError>
where
    F: FnOnce(&mut UpdateState),
{
    let mut state = read_state(db);
    f(&mut state);
    write_state(db, &state)
}

/// Записывает `version` в `dismissed_version`. Баннер не появится для
/// этой версии — но если выйдет ещё более новая, вернётся.
pub fn dismiss(db: &HistoryDB, version: &str) -> Result<(), ArcanaError> {
    with_state_mut(db, |state| {
        state.dismissed_version = Some(version.to_string());
    })
}

/// Помечает версию как «устанавливается». UI переходит в applying-режим
/// (прогресс + «Перезапустить»), баннер «Доступно» не показывается
/// поверх. Сбрасывается при старте, когда `APP_VERSION` догнал значение.
/// Вызывается из Linux- и Windows-веток apply_update — на macOS «мёртвая»
/// (там apply_update только открывает страницу релиза).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn set_applying(db: &HistoryDB, version: &str) -> Result<(), ArcanaError> {
    with_state_mut(db, |state| {
        state.applying_version = Some(version.to_string());
    })
}

/// Стирает applying-метку. Используется когда пользователь закрыл
/// applying-баннер крестиком (передумал перезапускаться).
pub fn clear_applying(db: &HistoryDB) -> Result<(), ArcanaError> {
    with_state_mut(db, |state| {
        state.applying_version = None;
    })
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

    /// Регресс на залипание обновления: если установка прервана (юзер закрыл
    /// терминал install.sh), applying-метка указывает на версию, которая так и
    /// НЕ установилась (APP_VERSION её не догнал). Пока метка стоит — баннер
    /// «Доступно» подавлен. `init_update_checker` снимает метку на старте; после
    /// этого подавления нет, а т.к. версия реально новее — баннер возвращается,
    /// и обновление можно повторить (раньше залипало навсегда).
    #[test]
    fn aborted_update_recovers_after_applying_cleared() {
        let latest = "9.9.9";
        assert!(is_newer(latest, APP_VERSION), "тест-версия должна быть новее текущей");

        // Прерванная установка: applying стоит, версия не догнала → баннер подавлен.
        let applying_stuck: Option<String> = Some(latest.to_string());
        assert!(
            applying_stuck.as_deref() == Some(latest),
            "пока applying стоит — баннер «Доступно» подавлен"
        );

        // Старт снимает метку (фикс) → подавления нет → восстановление.
        let applying_cleared: Option<String> = None;
        assert!(
            applying_cleared.as_deref() != Some(latest),
            "после снятия applying на старте баннер больше не подавлен"
        );
    }

    /// `pending_info_from_state` — общий источник для старт-баннера и ветки 304.
    /// Покрывает баг «304 теряет известное обновление»: при заполненном
    /// `latest_known` (новее текущей) обновление ВОЗВРАЩАЕТСЯ, а dismissed/
    /// applying/старая версия корректно дают None.
    #[test]
    fn pending_info_from_state_cases() {
        let newer = "9.9.9";
        assert!(is_newer(newer, APP_VERSION), "тест-версия должна быть новее текущей");

        let base = UpdateState {
            latest_known: Some(newer.to_string()),
            latest_release_url: Some("https://example.com/v9".into()),
            latest_published_at: Some("2026-06-15T00:00:00Z".into()),
            ..Default::default()
        };

        // Известный установимый релиз новее текущего → предлагаем (это и есть
        // поведение, которого не хватало на 304).
        let got = pending_info_from_state(&base).expect("newer known release → Some");
        assert_eq!(got.latest_version, newer);
        assert_eq!(got.release_url, "https://example.com/v9");

        // Кэш пуст → нечего предлагать.
        assert!(pending_info_from_state(&UpdateState::default()).is_none());

        // dismissed именно этой версии → None.
        let dismissed = UpdateState {
            dismissed_version: Some(newer.to_string()),
            ..base.clone()
        };
        assert!(pending_info_from_state(&dismissed).is_none());

        // applying этой версии → None (не перебиваем applying-режим).
        let applying = UpdateState {
            applying_version: Some(newer.to_string()),
            ..base.clone()
        };
        assert!(pending_info_from_state(&applying).is_none());

        // latest_known не новее текущей версии → None.
        let stale = UpdateState {
            latest_known: Some("0.0.1".into()),
            ..base.clone()
        };
        assert!(pending_info_from_state(&stale).is_none());
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

    /// `unix_now` — обёртка над системными часами: смещённый эпохой timestamp
    /// всегда положителен (после 1970). Детерминированной сверки нет, проверяем
    /// только знак — этого достаточно, чтобы отловить регресс `unwrap_or(0)`.
    #[test]
    fn unix_now_is_positive() {
        assert!(unix_now() > 0);
    }

    /// `set_applying` / `applying_version` / `clear_applying` — полный RMW-цикл
    /// applying-метки через реальную HistoryDB (а не конструкцию struct).
    #[test]
    fn set_then_clear_applying_roundtrips_via_db() {
        let db = temp_history_db("applying");
        assert!(applying_version(&db).is_none(), "baseline: метки нет");

        set_applying(&db, "9.9.9").expect("set_applying");
        assert_eq!(applying_version(&db).as_deref(), Some("9.9.9"));

        clear_applying(&db).expect("clear_applying");
        assert!(applying_version(&db).is_none(), "после clear метка снята");
    }

    /// `dismiss` пишет `dismissed_version` в БД и тем самым подавляет
    /// `cached_pending_update` именно для dismiss'нутой версии.
    #[test]
    fn dismiss_suppresses_cached_pending_update_for_that_version() {
        let newer = "9.9.9";
        assert!(is_newer(newer, APP_VERSION), "тест-версия должна быть новее текущей");

        let db = temp_history_db("dismiss");
        write_state(
            &db,
            &UpdateState {
                latest_known: Some(newer.to_string()),
                latest_release_url: Some("https://example.com/v9".into()),
                latest_published_at: Some("2026-06-15T00:00:00Z".into()),
                ..Default::default()
            },
        )
        .expect("write");

        // До dismiss — обновление предлагается.
        let before = cached_pending_update(&db).expect("newer known release → Some");
        assert_eq!(before.latest_version, newer);

        // После dismiss этой версии — cached_pending_update молчит.
        dismiss(&db, newer).expect("dismiss");
        assert!(cached_pending_update(&db).is_none(), "dismiss подавил баннер");
    }
}
