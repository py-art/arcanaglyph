// crates/arcanaglyph-core/src/engine/record_session.rs
//
// Фоновый пайплайн «запись → транскрибация → финализация» и хелперы старта,
// вынесенные из `trigger`. Поведение перенесено дословно — это структурное
// разбиение god-файла `engine.rs`, не изменение логики.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::time::Instant;
use tokio::sync::broadcast;
use tracing::info;

use super::{ArcanaEngine, EngineEvent};
use crate::audio::{self, AudioCommand};
use crate::config::CoreConfig;
use crate::error::ArcanaError;
use crate::history::HistoryDB;
use crate::transcriber::Transcriber;

/// Решение о доставке распознанного текста (чистая логика, тестируемо).
/// `should_type` — вставлять ли текст в активное окно (только когда наше окно
/// скрыто в трее). `should_focus` — запросить ли фокус (когда окно видимо).
struct Delivery {
    should_type: bool,
    should_focus: bool,
}

/// Чистое решение по доставке: печатать только при скрытом окне, фокус — при видимом.
fn classify_delivery(auto_type: bool, is_visible: bool) -> Delivery {
    Delivery {
        should_type: auto_type && !is_visible,
        should_focus: is_visible,
    }
}

/// Текст для записи в историю: с удалением слов-паразитов или без. Чистая функция.
fn build_history_entry(rec_text: &str, remove_fillers: bool) -> String {
    if remove_fillers {
        crate::transcriber::remove_filler_words(rec_text)
    } else {
        rec_text.to_string()
    }
}

/// Регистрирует только что созданную модель: кладёт в пул, делает активной,
/// засекает `last_used` (иначе sweeper мог бы выгрузить её на следующем тике) и
/// возвращает UI в «готов»-состояние. Вынесено из `resolve_reloaded_transcriber`.
fn register_reloaded_model(
    name: &str,
    t: &Arc<dyn Transcriber>,
    transcribers_pool: &Arc<std::sync::RwLock<HashMap<String, Arc<dyn Transcriber>>>>,
    active_model_rw: &Arc<std::sync::RwLock<String>>,
    last_used_arc: &Arc<std::sync::RwLock<HashMap<String, Instant>>>,
    event_tx: &broadcast::Sender<EngineEvent>,
) {
    if let Ok(mut pool) = transcribers_pool.write() {
        pool.insert(name.to_string(), t.clone());
    }
    if let Ok(mut active) = active_model_rw.write() {
        *active = name.to_string();
    }
    if let Ok(mut last) = last_used_arc.write() {
        last.insert(name.to_string(), Instant::now());
    }
    info!("Модель '{}' загружена и добавлена в пул", name);
    let _ = event_tx.send(EngineEvent::ModelLoaded);
}

/// Разрешает транскрайбер под текущий конфиг при `need_reload`. Берёт
/// предзагруженную модель из пула либо лениво создаёт её в фоне (с теми же
/// loading-событиями, что и `preload_model`). Возвращает `None`, если загрузка
/// провалилась (Error уже эмитнут) — вызывающий должен прервать запись.
/// Вынесено из `trigger` дословно — поведение не меняется.
#[allow(clippy::too_many_arguments)]
pub(super) async fn resolve_reloaded_transcriber(
    need_reload: bool,
    current: Arc<dyn Transcriber>,
    config: &CoreConfig,
    transcribers_rw: &Arc<std::sync::RwLock<HashMap<String, Arc<dyn Transcriber>>>>,
    active_model_rw: &Arc<std::sync::RwLock<String>>,
    last_used_arc: &Arc<std::sync::RwLock<HashMap<String, Instant>>>,
    event_tx: &broadcast::Sender<EngineEvent>,
) -> Option<Arc<dyn Transcriber>> {
    if !need_reload {
        return Some(current);
    }

    // Проверяем, может модель уже в пуле (предзагружена)
    let target_name = config.transcriber_model_name();
    let already_in_pool = transcribers_rw
        .read()
        .map(|pool| pool.get(&target_name).cloned())
        .unwrap_or(None);

    if let Some(t) = already_in_pool {
        if let Ok(mut active) = active_model_rw.write() {
            *active = target_name.clone();
        }
        info!("Модель '{}' взята из пула (была предзагружена)", target_name);
        return Some(t);
    }

    // Lazy-fallback: модели ещё нет в пуле (eager preload в save_config не успел
    // или конфиг был изменён внешне). Эмитим тот же loading-event, что и
    // preload_model — UI отработает одинаково.
    let _ = event_tx.send(EngineEvent::ModelLoading(target_name.clone()));
    let cfg = config.clone();
    let transcribers_pool = Arc::clone(transcribers_rw);
    let reload_result =
        tokio::task::spawn_blocking(move || ArcanaEngine::create_transcriber(&cfg, &cfg.transcriber)).await;

    match reload_result {
        Ok(Ok((name, t))) => {
            register_reloaded_model(&name, &t, &transcribers_pool, active_model_rw, last_used_arc, event_tx);
            Some(t)
        }
        Ok(Err(e)) => {
            tracing::error!("Не удалось пересоздать транскрайбер: {}", e);
            let _ = event_tx.send(EngineEvent::Error(format!("Ошибка загрузки модели: {}", e)));
            None
        }
        Err(e) => {
            tracing::error!("Ошибка загрузки модели: {:?}", e);
            None
        }
    }
}

/// Параметры фоновой задачи «запись → транскрибация → финализация».
/// Всё owned — структура целиком переносится в async-move задачу из `trigger`.
pub(super) struct RecordSession {
    pub(super) transcriber: Arc<dyn Transcriber>,
    pub(super) history_db: Arc<HistoryDB>,
    pub(super) event_tx: broadcast::Sender<EngineEvent>,
    pub(super) audio_level: Arc<AtomicU32>,
    pub(super) window_visible: Arc<AtomicBool>,
    pub(super) is_busy: Arc<tokio::sync::Mutex<bool>>,
    pub(super) is_paused: Arc<tokio::sync::Mutex<bool>>,
    pub(super) cmd_tx_cleanup: Arc<tokio::sync::Mutex<Option<std_mpsc::Sender<AudioCommand>>>>,
    pub(super) config: CoreConfig,
    pub(super) sample_rate: u32,
    pub(super) debug: bool,
    pub(super) silence_timeout_secs: u64,
    pub(super) vad_enabled: bool,
    pub(super) vad_silence_secs: u64,
    pub(super) mic_gain: f32,
    pub(super) auto_type: bool,
    pub(super) remove_fillers: bool,
}

/// Обработка успешного результата записи: пост-процессинг текста, запись в
/// историю и доставка (вставка в активное окно / запрос фокуса). Вынесено из
/// `run_record_session` для снижения cc и вложенности — поведение не меняется.
#[allow(clippy::too_many_arguments)]
async fn finalize_recording(
    rec: crate::audio::RecordResult,
    remove_fillers: bool,
    config: &CoreConfig,
    history_db: &Arc<HistoryDB>,
    window_visible: &Arc<AtomicBool>,
    auto_type: bool,
    event_tx: &broadcast::Sender<EngineEvent>,
) {
    // Пост-процессинг: удаление слов-паразитов
    let text = build_history_entry(&rec.text, remove_fillers);

    if text.is_empty() {
        tracing::warn!("Распознавание вернуло пустой результат. Проверьте микрофон.");
        let _ = event_tx.send(EngineEvent::Error(
            "Микрофон не захватил речь. Проверьте, что микрофон подключён и выбран как устройство по умолчанию."
                .to_string(),
        ));
        return;
    }

    // Сохраняем в историю
    let model_name = config.transcriber_model_name();
    let transcriber_type_str = config.transcriber_type_str();
    if let Err(e) = (|| -> Result<(), crate::error::ArcanaError> {
        let rec_id = history_db.add_recording(&rec.audio_path, rec.duration_secs)?;
        history_db.add_transcription(rec_id, &text, &model_name, &transcriber_type_str)?;
        Ok(())
    })() {
        tracing::warn!("Не удалось сохранить в историю: {}", e);
    }

    let is_visible = window_visible.load(Ordering::Relaxed);
    let delivery = classify_delivery(auto_type, is_visible);
    // Вставляем текст только когда окно скрыто (в трее)
    if delivery.should_type {
        eprintln!("[Вставка] в активное окно...");
        if let Err(e) = crate::input::type_text(&text).await {
            tracing::error!("Не удалось вставить текст: {}", e);
        }
    }
    let _ = event_tx.send(EngineEvent::TranscriptionResult(text));
    // Если окно видимо — запрашиваем фокус для вывода на передний план
    if delivery.should_focus {
        let _ = event_tx.send(EngineEvent::RequestFocus);
    }
}

/// Фоновая запись + транскрибация + финализация: пишет в историю, вставляет текст,
/// шлёт события, сбрасывает busy/paused. Вынесено из `trigger` — снимает самую
/// глубокую вложенность; тело перенесено дословно (поведение не меняется).
pub(super) async fn run_record_session(session: RecordSession, cmd_rx: std_mpsc::Receiver<AudioCommand>) {
    let RecordSession {
        transcriber,
        history_db,
        event_tx,
        audio_level,
        window_visible,
        is_busy,
        is_paused,
        cmd_tx_cleanup,
        config,
        sample_rate,
        debug,
        silence_timeout_secs,
        vad_enabled,
        vad_silence_secs,
        mic_gain,
        auto_type,
        remove_fillers,
    } = session;

    let event_tx_audio = event_tx.clone();
    let audio_cache = history_db.audio_cache_path().to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        let params = audio::RecordParams {
            sample_rate,
            debug,
            silence_timeout_secs,
            vad_enabled,
            vad_silence_secs,
            mic_gain,
            audio_cache_dir: audio_cache,
        };
        let channels = audio::RecordChannels {
            cmd_rx,
            audio_level,
            event_tx: event_tx_audio,
        };
        audio::record_and_transcribe(params, channels, transcriber.as_ref())
    })
    .await;

    match result {
        Ok(Ok(rec)) => {
            finalize_recording(
                rec,
                remove_fillers,
                &config,
                &history_db,
                &window_visible,
                auto_type,
                &event_tx,
            )
            .await;
        }
        Ok(Err(ArcanaError::Cancelled)) => {
            // Пользователь нажал «Стоп» во время инференса —
            // не ошибка, тихо завершаем без error-toast.
            tracing::info!("Транскрибация отменена пользователем");
        }
        Ok(Err(e @ ArcanaError::AudioDevice(_))) => {
            // Мёртвый/молчащий микрофон (грейс-окно живости в record_and_transcribe).
            // Раньше это ловил pre-flight `check_mic_or_abort`; сохраняем его UX —
            // при скрытом окне вставляем текст ошибки в активное поле, иначе
            // пользователь не увидит ничего (плашка в трее).
            let msg = e.to_string();
            tracing::error!("{}", msg);
            eprintln!("[Ошибка] {}", msg);
            if !window_visible.load(Ordering::Relaxed) {
                let _ = crate::input::type_text(&format!("[Ошибка микрофона] {}", msg)).await;
            }
            let _ = event_tx.send(EngineEvent::Error(msg));
        }
        Ok(Err(e)) => {
            tracing::error!("Ошибка транскрибации: {}", e);
            let _ = event_tx.send(EngineEvent::Error(format!("Ошибка транскрибации: {}", e)));
        }
        Err(e) => {
            tracing::error!("Задача записи завершилась с ошибкой: {:?}", e);
            let _ = event_tx.send(EngineEvent::Error(format!("Ошибка записи: {:?}", e)));
        }
    }

    info!("Обработка завершена. Система готова к новой записи.");
    let _ = event_tx.send(EngineEvent::FinishedProcessing);

    *is_busy.lock().await = false;
    *is_paused.lock().await = false;
    *cmd_tx_cleanup.lock().await = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_delivery() {
        // Окно скрыто + auto_type → печатаем, без фокуса.
        let d = classify_delivery(true, false);
        assert!(d.should_type && !d.should_focus);
        // Окно видимо → не печатаем, запрашиваем фокус.
        let d = classify_delivery(true, true);
        assert!(!d.should_type && d.should_focus);
        // auto_type выключен, окно скрыто → ничего.
        let d = classify_delivery(false, false);
        assert!(!d.should_type && !d.should_focus);
    }

    #[test]
    fn test_build_history_entry() {
        assert_eq!(build_history_entry("э привет", true), "привет");
        assert_eq!(build_history_entry("э привет", false), "э привет");
    }
}
