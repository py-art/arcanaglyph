// crates/arcanaglyph-core/src/engine/lru.rs
//
// LRU-выгрузка простаивающих моделей из пула транскрайберов: чистая логика
// отбора кандидатов + фоновый sweeper. Вынесено из `engine.rs` дословно —
// структурное разбиение god-файла, поведение не меняется.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::info;

use super::ArcanaEngine;
use crate::config::CoreConfig;
use crate::transcriber::Transcriber;

/// TTL выгрузки из минут конфигурации. `0` → `None` (sweeper отключён).
fn ttl_from_minutes(ttl_min: u64) -> Option<Duration> {
    if ttl_min == 0 {
        None
    } else {
        Some(Duration::from_secs(ttl_min * 60))
    }
}

/// Чистый отбор моделей на LRU-выгрузку. Пропускает: активную модель, модели с
/// idle < `ttl`, и те, на которые есть внешняя ссылка (`strong_count > 1` — Arc
/// держится не только пулом, значит идёт инференс).
fn lru_eviction_candidates(
    pool: &HashMap<String, Arc<dyn Transcriber>>,
    last_used: &HashMap<String, Instant>,
    active_name: &str,
    now: Instant,
    ttl: Duration,
) -> Vec<String> {
    pool.iter()
        .filter_map(|(name, arc)| {
            if name == active_name {
                return None;
            }
            let used = last_used.get(name).copied().unwrap_or(now);
            if now.duration_since(used) < ttl {
                return None;
            }
            if Arc::strong_count(arc) > 1 {
                return None;
            }
            Some(name.clone())
        })
        .collect()
}

/// Write-фаза LRU-выгрузки: удаляет кандидатов из пула и `last_used`, перепроверяя
/// активную модель и `strong_count` (между read- и write-локом могло измениться).
fn evict_candidates(
    pool: &mut HashMap<String, Arc<dyn Transcriber>>,
    last_used: &mut HashMap<String, Instant>,
    candidates: Vec<String>,
    active_name_now: &str,
    ttl_min: u64,
) {
    for name in candidates {
        if name == active_name_now {
            continue;
        }
        if let Some(arc) = pool.get(&name)
            && Arc::strong_count(arc) > 1
        {
            continue;
        }
        pool.remove(&name);
        last_used.remove(&name);
        info!("Модель '{}' выгружена по LRU (idle ≥ {} мин)", name, ttl_min);
    }
}

impl ArcanaEngine {
    /// Запускает sweeper в `rt_handle`: раз в минуту читает текущий TTL из
    /// `config.model_unload_after_minutes`, выгружает модели, простаивающие
    /// дольше N минут. Никогда не выгружает активную модель. Защищён от гонки
    /// с инференсом через `is_busy.try_lock` (если занят — пропускаем тик)
    /// и через `Arc::strong_count(&transcriber) == 1` (нет внешних ссылок).
    pub(super) fn spawn_lru_sweeper(&self) {
        let transcribers = Arc::clone(&self.transcribers);
        let last_used = Arc::clone(&self.last_used);
        let active_model = Arc::clone(&self.active_model);
        let is_busy = Arc::clone(&self.is_busy);
        // config читаем напрямую через clone каждый тик через метод? Нет, нужно
        // прокинуть Arc — но config: RwLock<CoreConfig>, не Arc<RwLock<...>>.
        // Чтобы избежать рефакторинга всего поля, читаем через CoreConfig::load
        // (то же самое значение из SQLite settings, синхронизируется с UI через
        // save_config). Цена: один SQL SELECT раз в минуту — пренебрежимо мала.
        self.rt_handle.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            // Первый tick срабатывает сразу — пропускаем (engine только что создан).
            interval.tick().await;
            loop {
                interval.tick().await;

                let ttl_min = CoreConfig::load().map(|c| c.model_unload_after_minutes).unwrap_or(0);
                let Some(ttl) = ttl_from_minutes(ttl_min) else {
                    continue; // TTL = 0 → sweeper отключён
                };

                // Если идёт инференс — пропускаем тик. Это устраняет окно гонки
                // «transcriber взят, но Arc передан в spawn_blocking → strong_count
                // временно >1». Try_lock неблокирующий: если занято, просто ждём
                // следующий тик через минуту.
                if is_busy.try_lock().is_err() {
                    tracing::debug!("LRU sweeper: занято инференсом, пропускаю тик");
                    continue;
                }

                let active_name = active_model.read().map(|m| m.clone()).unwrap_or_default();
                let now = Instant::now();

                // Собираем кандидатов под read-lock'ом, освобождаем его, потом берём
                // write-lock. Не атомарно с infer'ом, но он забусен (is_busy выше).
                let candidates: Vec<String> = {
                    let pool = match transcribers.read() {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    let last = match last_used.read() {
                        Ok(l) => l,
                        Err(_) => continue,
                    };
                    lru_eviction_candidates(&pool, &last, &active_name, now, ttl)
                };

                if candidates.is_empty() {
                    continue;
                }

                // Write-фаза: перепроверяем active/strong_count (между read и write
                // могло измениться) и удаляем.
                let active_name_now = active_model.read().map(|m| m.clone()).unwrap_or_default();
                if let (Ok(mut pool), Ok(mut last)) = (transcribers.write(), last_used.write()) {
                    evict_candidates(&mut pool, &mut last, candidates, &active_name_now, ttl_min);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Заглушка Transcriber для тестов LRU-логики (без реальной модели).
    struct DummyTranscriber;
    impl Transcriber for DummyTranscriber {
        fn transcribe(&self, _samples: &[i16], _sample_rate: u32) -> Result<String, crate::error::ArcanaError> {
            Ok(String::new())
        }
        fn supports_streaming(&self) -> bool {
            false
        }
    }

    #[test]
    fn test_ttl_from_minutes() {
        assert!(ttl_from_minutes(0).is_none());
        assert_eq!(ttl_from_minutes(5), Some(Duration::from_secs(300)));
    }

    #[test]
    fn test_lru_eviction_candidates_basic() {
        let mut pool: HashMap<String, Arc<dyn Transcriber>> = HashMap::new();
        pool.insert("whisper".into(), Arc::new(DummyTranscriber));
        pool.insert("gigaam".into(), Arc::new(DummyTranscriber));
        let now = Instant::now();
        let mut last: HashMap<String, Instant> = HashMap::new();
        last.insert("whisper".into(), now - Duration::from_secs(600)); // idle 10 мин
        last.insert("gigaam".into(), now - Duration::from_secs(1));
        // active = gigaam, ttl = 5 мин → выгружается только whisper.
        let cands = lru_eviction_candidates(&pool, &last, "gigaam", now, Duration::from_secs(300));
        assert_eq!(cands, vec!["whisper".to_string()]);
    }

    #[test]
    fn test_lru_eviction_skips_externally_referenced() {
        let mut pool: HashMap<String, Arc<dyn Transcriber>> = HashMap::new();
        let whisper: Arc<dyn Transcriber> = Arc::new(DummyTranscriber);
        let _external = Arc::clone(&whisper); // strong_count > 1 → идёт инференс
        pool.insert("whisper".into(), whisper);
        let now = Instant::now();
        let mut last: HashMap<String, Instant> = HashMap::new();
        last.insert("whisper".into(), now - Duration::from_secs(600));
        let cands = lru_eviction_candidates(&pool, &last, "gigaam", now, Duration::from_secs(300));
        assert!(cands.is_empty());
    }
}
