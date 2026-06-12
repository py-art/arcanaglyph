// crates/arcanaglyph-core/src/engine.rs

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tracing::info;

use crate::audio::{self, AudioCommand};
use crate::config::{CoreConfig, TranscriberType};
use crate::error::ArcanaError;
// Backend GigaAM: один transcriber.rs через ort, отличается только способ доставки
// libonnxruntime.so (см. core/Cargo.toml):
// - feature `gigaam` → ort + Microsoft pre-built ONNX (требует AVX)
// - feature `gigaam-system-ort` → ort + локально собранная libonnxruntime (без AVX)
#[cfg(any(feature = "gigaam", feature = "gigaam-system-ort"))]
use crate::gigaam::transcriber::GigaAmTranscriber;
use crate::history::HistoryDB;
#[cfg(feature = "qwen3asr")]
use crate::qwen3asr::transcriber::Qwen3AsrTranscriber;
use crate::transcriber::Transcriber;
#[cfg(feature = "vosk")]
use crate::transcriber::VoskTranscriber;
#[cfg(feature = "whisper")]
use crate::transcriber::WhisperTranscriber;

/// События движка, рассылаемые подписчикам
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Запись началась
    RecordingStarted,
    /// Запись приостановлена
    RecordingPaused,
    /// Запись возобновлена
    RecordingResumed,
    /// Результат транскрибации
    TranscriptionResult(String),
    /// Транскрибация началась (запись завершена, идёт распознавание)
    Transcribing,
    /// Обработка завершена, система готова к новой записи
    FinishedProcessing,
    /// Запрос на вывод окна на передний план (когда окно видимо)
    RequestFocus,
    /// Начата загрузка модели в память (eager-preload из save_config или lazy-fallback в trigger).
    /// Payload — отображаемое имя модели для UI ("Vosk Russian 0.42").
    ModelLoading(String),
    /// Модель загружена, приложение готово к работе
    ModelLoaded,
    /// Ошибка, которую нужно показать пользователю
    Error(String),
}

/// Основной движок ArcanaGlyph: управляет записью, распознаванием и рассылкой событий
pub struct ArcanaEngine {
    config: std::sync::RwLock<CoreConfig>,
    /// Пул загруженных транскрайберов (ключ — имя модели)
    transcribers: Arc<std::sync::RwLock<HashMap<String, Arc<dyn Transcriber>>>>,
    /// Имя активной модели (ключ в transcribers)
    active_model: Arc<std::sync::RwLock<String>>,
    /// Когда модель использовалась в последний раз — для LRU-выгрузки sweeper'ом.
    /// Ключи синхронизированы с `transcribers`: запись добавляется при `preload`
    /// или первом `trigger`, удаляется одновременно с моделью из пула.
    last_used: Arc<std::sync::RwLock<HashMap<String, Instant>>>,
    config_changed: AtomicBool,
    is_busy: Arc<tokio::sync::Mutex<bool>>,
    is_paused: Arc<tokio::sync::Mutex<bool>>,
    current_cmd_tx: Arc<tokio::sync::Mutex<Option<std_mpsc::Sender<AudioCommand>>>>,
    event_tx: broadcast::Sender<EngineEvent>,
    audio_level: Arc<AtomicU32>,
    rt_handle: Handle,
    /// Флаг видимости окна: true — окно видимо, false — свёрнуто в трей
    window_visible: Arc<AtomicBool>,
    /// База данных истории транскрибаций
    history_db: Arc<HistoryDB>,
}

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

/// Маппинг результата `spawn_blocking(check_microphone)` в текст ошибки.
/// `None` — микрофон в порядке. Чистая функция (тестируется через `Ok(Ok(()))`).
fn mic_error_message(mic_check: Result<Result<(), ArcanaError>, tokio::task::JoinError>) -> Option<String> {
    match mic_check {
        Ok(Err(e)) => Some(e.to_string()),
        Err(e) => Some(format!("Ошибка проверки микрофона: {:?}", e)),
        Ok(Ok(())) => None,
    }
}

/// Параметры фоновой задачи «запись → транскрибация → финализация».
/// Всё owned — структура целиком переносится в async-move задачу из `trigger`.
struct RecordSession {
    transcriber: Arc<dyn Transcriber>,
    history_db: Arc<HistoryDB>,
    event_tx: broadcast::Sender<EngineEvent>,
    audio_level: Arc<AtomicU32>,
    window_visible: Arc<AtomicBool>,
    is_busy: Arc<tokio::sync::Mutex<bool>>,
    is_paused: Arc<tokio::sync::Mutex<bool>>,
    cmd_tx_cleanup: Arc<tokio::sync::Mutex<Option<std_mpsc::Sender<AudioCommand>>>>,
    config: CoreConfig,
    sample_rate: u32,
    debug: bool,
    silence_timeout_secs: u64,
    vad_enabled: bool,
    vad_silence_secs: u64,
    mic_gain: f32,
    auto_type: bool,
    remove_fillers: bool,
}

/// Фоновая запись + транскрибация + финализация: пишет в историю, вставляет текст,
/// шлёт события, сбрасывает busy/paused. Вынесено из `trigger` — снимает самую
/// глубокую вложенность; тело перенесено дословно (поведение не меняется).
async fn run_record_session(session: RecordSession, cmd_rx: std_mpsc::Receiver<AudioCommand>) {
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
            // Пост-процессинг: удаление слов-паразитов
            let text = build_history_entry(&rec.text, remove_fillers);

            if text.is_empty() {
                tracing::warn!("Распознавание вернуло пустой результат. Проверьте микрофон.");
                let _ = event_tx.send(EngineEvent::Error(
                    "Микрофон не захватил речь. Проверьте, что микрофон подключён и выбран как устройство по умолчанию.".to_string(),
                ));
            } else {
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
        }
        Ok(Err(crate::error::ArcanaError::Cancelled)) => {
            // Пользователь нажал «Стоп» во время инференса —
            // не ошибка, тихо завершаем без error-toast.
            tracing::info!("Транскрибация отменена пользователем");
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

impl ArcanaEngine {
    /// Создаёт новый экземпляр движка: загружает модель, инициализирует каналы.
    /// Должен вызываться из контекста Tokio runtime (сохраняет Handle для spawn).
    pub fn new(config: CoreConfig, window_visible: Arc<AtomicBool>) -> Result<Self, ArcanaError> {
        // ort инициализируется лениво в `create_transcriber` под плечом ONNX-движка —
        // чтобы не дёргать AVX-инструкции, если активный движок не GigaAM/Qwen3-ASR.
        // Загружаем основную модель
        let (model_name, transcriber) = Self::create_transcriber(&config, &config.transcriber)?;

        let mut transcribers = HashMap::new();
        transcribers.insert(model_name.clone(), transcriber);

        // Инициализация БД истории
        let db_path = CoreConfig::history_db_path()
            .ok_or_else(|| ArcanaError::Database("Не удалось определить путь к БД истории".into()))?;
        let audio_cache = CoreConfig::audio_cache_dir()
            .ok_or_else(|| ArcanaError::Database("Не удалось определить путь к кэшу аудио".into()))?;
        let history_db = Arc::new(HistoryDB::new(&db_path, audio_cache)?);

        let (event_tx, _) = broadcast::channel::<EngineEvent>(32);

        let rt_handle = Handle::try_current().map_err(|_| {
            ArcanaError::Internal("ArcanaEngine::new() должен вызываться из контекста Tokio runtime".into())
        })?;

        // Стартовое значение last_used для основной (активной) модели — сейчас.
        // Sweeper никогда не выгружает активную, но запись нужна чтобы при смене
        // активной модели (через update_config) у предыдущей был корректный idle.
        let mut last_used = HashMap::new();
        last_used.insert(model_name.clone(), Instant::now());

        let engine = Self {
            config: std::sync::RwLock::new(config),
            transcribers: Arc::new(std::sync::RwLock::new(transcribers)),
            active_model: Arc::new(std::sync::RwLock::new(model_name)),
            last_used: Arc::new(std::sync::RwLock::new(last_used)),
            config_changed: AtomicBool::new(false),
            is_busy: Arc::new(tokio::sync::Mutex::new(false)),
            is_paused: Arc::new(tokio::sync::Mutex::new(false)),
            current_cmd_tx: Arc::new(tokio::sync::Mutex::new(None)),
            event_tx,
            audio_level: Arc::new(AtomicU32::new(0)),
            rt_handle,
            window_visible,
            history_db,
        };

        // Фоновый sweeper: раз в минуту проверяем неактивные модели и выгружаем
        // из пула те, что простаивали дольше `config.model_unload_after_minutes`.
        // Отключается на 0 — настройка читается каждый тик (hot-reload).
        engine.spawn_lru_sweeper();

        Ok(engine)
    }

    /// Запускает sweeper в `rt_handle`: раз в минуту читает текущий TTL из
    /// `config.model_unload_after_minutes`, выгружает модели, простаивающие
    /// дольше N минут. Никогда не выгружает активную модель. Защищён от гонки
    /// с инференсом через `is_busy.try_lock` (если занят — пропускаем тик)
    /// и через `Arc::strong_count(&transcriber) == 1` (нет внешних ссылок).
    fn spawn_lru_sweeper(&self) {
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

    /// Подавляет verbose-логи ONNX Runtime до создания первой сессии.
    /// Вызывается лениво — только когда реально создаётся ONNX-транскрайбер.
    /// Не активна для `gigaam-system-ort`: там `Environment::current()` до создания
    /// сессии может зависнуть в load-dynamic пути (см. transcriber инициализацию).
    #[cfg(any(feature = "gigaam", feature = "qwen3asr"))]
    fn init_ort_logging() {
        if let Ok(env) = ort::environment::Environment::current() {
            env.set_log_level(ort::logging::LogLevel::Warning);
        }
    }

    /// Создаёт транскрайбер по типу, возвращает (model_name, Arc<dyn Transcriber>).
    ///
    /// `allow(unused_variables)` — для сборок без ни одного движка все плечи `match` стираются,
    /// и параметр `config` становится формально неиспользуемым.
    #[allow(unused_variables)]
    fn create_transcriber(
        config: &CoreConfig,
        t_type: &TranscriberType,
    ) -> Result<(String, Arc<dyn Transcriber>), ArcanaError> {
        match t_type {
            #[cfg(feature = "vosk")]
            TranscriberType::Vosk => {
                let t = VoskTranscriber::new(&config.model_path, config.sample_rate as f32)?;
                let name = config
                    .model_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "vosk".to_string());
                Ok((name, Arc::new(t)))
            }
            #[cfg(feature = "whisper")]
            TranscriberType::Whisper => {
                let t = WhisperTranscriber::new(&config.whisper_model_path)?;
                let name = config
                    .whisper_model_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "whisper".to_string());
                Ok((name, Arc::new(t)))
            }
            #[cfg(any(feature = "gigaam", feature = "gigaam-system-ort"))]
            TranscriberType::GigaAm => {
                // init_ort_logging() ВРЕМЕННО только для feature `gigaam` (статически
                // линкованный ORT). Для `gigaam-system-ort` (load-dynamic) вызов
                // Environment::current() до dlopen сессии может зависнуть — пропускаем.
                #[cfg(feature = "gigaam")]
                Self::init_ort_logging();
                let t = GigaAmTranscriber::new(&config.gigaam_model_path)?;
                let name = config
                    .gigaam_model_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "gigaam".to_string());
                Ok((name, Arc::new(t)))
            }
            #[cfg(feature = "qwen3asr")]
            TranscriberType::Qwen3Asr => {
                Self::init_ort_logging();
                let t = Qwen3AsrTranscriber::new(&config.qwen3asr_model_path)?;
                let name = config
                    .qwen3asr_model_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "qwen3asr".to_string());
                Ok((name, Arc::new(t)))
            }
            // Любая ветка без своего feature: сообщаем, что движок недоступен.
            #[allow(unreachable_patterns)]
            other => Err(ArcanaError::EngineNotAvailable(other.as_str().to_string())),
        }
    }

    /// Предзагрузить модель в пул (вызывать из фонового потока).
    /// Пропускает загрузку если модель уже в пуле.
    pub fn preload_model(&self, t_type: &TranscriberType) -> Result<String, ArcanaError> {
        let config = self
            .config
            .read()
            .map_err(|e| ArcanaError::Internal(format!("RwLock: {}", e)))?;

        // Определяем имя модели без загрузки — чтобы проверить пул
        let expected_name = match t_type {
            TranscriberType::Vosk => config
                .model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "vosk".to_string()),
            TranscriberType::Whisper => config
                .whisper_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "whisper".to_string()),
            TranscriberType::GigaAm => config
                .gigaam_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "gigaam".to_string()),
            TranscriberType::Qwen3Asr => config
                .qwen3asr_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "qwen3asr".to_string()),
        };

        // Если модель уже в пуле — пропускаем загрузку
        let already_loaded = self
            .transcribers
            .read()
            .map(|pool| pool.contains_key(&expected_name))
            .unwrap_or(false);
        if already_loaded {
            return Ok(expected_name);
        }

        // Эмитим событие "началась загрузка" — UI заменит top-status на «Загрузка модели N…»
        // и заблокирует mic-btn.
        let _ = self.event_tx.send(EngineEvent::ModelLoading(expected_name.clone()));

        let (name, transcriber) = Self::create_transcriber(&config, t_type)?;
        let mut pool = self
            .transcribers
            .write()
            .map_err(|e| ArcanaError::Internal(format!("RwLock: {}", e)))?;
        pool.insert(name.clone(), transcriber);
        // LRU: засекаем момент попадания модели в пул. Без этого свежезагруженная
        // модель могла бы попасть под немедленную выгрузку, если sweeper тикнет
        // раньше первого использования (теоретически возможно, если TTL = 0
        // только что переключили на 1 мин в Settings).
        if let Ok(mut last) = self.last_used.write() {
            last.insert(name.clone(), Instant::now());
        }
        info!("Модель '{}' предзагружена в пул", name);
        // Возвращаем UI в «готов»-состояние.
        let _ = self.event_tx.send(EngineEvent::ModelLoaded);
        Ok(name)
    }

    /// Список загруженных моделей
    pub fn loaded_models(&self) -> Vec<String> {
        self.transcribers
            .read()
            .map(|pool| pool.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Сколько секунд каждая загруженная модель не использовалась — для UI
    /// настройки LRU TTL («Загружено: GigaAM (idle 12с), Whisper (idle 3 мин)»).
    /// `Instant` не сериализуется через serde, поэтому конвертим в `u64`
    /// прямо здесь.
    pub fn loaded_models_idle_seconds(&self) -> HashMap<String, u64> {
        let now = Instant::now();
        match self.last_used.read() {
            Ok(last) => last
                .iter()
                .map(|(name, t)| (name.clone(), now.duration_since(*t).as_secs()))
                .collect(),
            Err(_) => HashMap::new(),
        }
    }

    /// Текущий активный тип транскрайбера (по конфигу).
    /// Используется снаружи (Tauri save_config) чтобы понять, нужна ли eager-preload
    /// после смены движка.
    pub fn active_transcriber_type(&self) -> crate::config::TranscriberType {
        self.config.read().map(|c| c.transcriber.clone()).unwrap_or_default()
    }

    /// Имя активной модели
    pub fn active_model_name(&self) -> String {
        self.active_model.read().map(|m| m.clone()).unwrap_or_default()
    }

    /// Обновить конфигурацию — если модель уже в пуле, мгновенное переключение
    pub fn update_config(&self, new_config: CoreConfig) {
        // Определяем имя модели для нового конфига
        let new_model_name = match new_config.transcriber {
            TranscriberType::Vosk => new_config
                .model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "vosk".to_string()),
            TranscriberType::Whisper => new_config
                .whisper_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "whisper".to_string()),
            TranscriberType::GigaAm => new_config
                .gigaam_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "gigaam".to_string()),
            TranscriberType::Qwen3Asr => new_config
                .qwen3asr_model_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "qwen3asr".to_string()),
        };

        // Если модель уже в пуле — мгновенное переключение
        let already_loaded = self
            .transcribers
            .read()
            .map(|pool| pool.contains_key(&new_model_name))
            .unwrap_or(false);

        if already_loaded {
            if let Ok(mut active) = self.active_model.write() {
                *active = new_model_name.clone();
            }
            // LRU: при мгновенном переключении обновляем last_used новой активной
            // модели — иначе sweeper мог бы её выгрузить, если она долго простаивала
            // в пуле и пользователь только что переключился именно на неё.
            if let Ok(mut last) = self.last_used.write() {
                last.insert(new_model_name.clone(), Instant::now());
            }
            info!("Модель '{}' уже загружена — мгновенное переключение", new_model_name);
        } else {
            self.config_changed.store(true, Ordering::Relaxed);
            info!(
                "Модель '{}' не в пуле — загрузится при следующей записи",
                new_model_name
            );
        }

        if let Ok(mut cfg) = self.config.write() {
            *cfg = new_config;
        }
    }

    /// Доступ к БД истории (для Tauri команд)
    pub fn history_db(&self) -> &Arc<HistoryDB> {
        &self.history_db
    }

    /// Подписаться на события движка
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.event_tx.subscribe()
    }

    /// Получить текущий уровень громкости (0-100)
    pub fn get_audio_level(&self) -> u32 {
        self.audio_level.load(Ordering::Relaxed)
    }

    /// Проверить, идёт ли сейчас запись
    pub async fn is_recording(&self) -> bool {
        *self.is_busy.lock().await
    }

    /// Проверить, на паузе ли запись
    pub async fn is_paused(&self) -> bool {
        *self.is_paused.lock().await
    }

    /// Показывать ли виджет записи (из текущего конфига)
    pub fn show_widget(&self) -> bool {
        self.config.read().map_or(true, |c| c.show_widget)
    }

    /// Переключатель записи: если не записывает — начать, если записывает — остановить.
    pub fn trigger(&self) {
        // Глобальный счётчик вызовов trigger() — для диагностики double-trigger
        // (см. main.rs UDP listener vs Tauri global-shortcut handler). В логах
        // call_id=N + was_busy позволяет понять: 2 быстрых вызова подряд start+stop?
        static TRIGGER_CALL_COUNT: AtomicU32 = AtomicU32::new(0);
        let call_id = TRIGGER_CALL_COUNT.fetch_add(1, Ordering::Relaxed);
        info!(trigger_call_id = call_id, "ArcanaEngine::trigger ENTRY (sync)");

        let is_busy = Arc::clone(&self.is_busy);
        let is_paused = Arc::clone(&self.is_paused);
        let current_cmd_tx = Arc::clone(&self.current_cmd_tx);
        let event_tx = self.event_tx.clone();
        let active_name = self.active_model.read().unwrap().clone();
        // LRU: обновляем last_used для активной модели — каждое нажатие Ctrl+Ё
        // отодвигает время выгрузки. Без этого активно используемая модель могла
        // бы попасть под выгрузку, если пользователь делает паузы > TTL между записями.
        if let Ok(mut last) = self.last_used.write() {
            last.insert(active_name.clone(), Instant::now());
        }
        let mut transcriber = self
            .transcribers
            .read()
            .unwrap()
            .get(&active_name)
            .cloned()
            .unwrap_or_else(|| self.transcribers.read().unwrap().values().next().unwrap().clone());
        let need_reload = self.config_changed.swap(false, Ordering::Relaxed);
        let config = self.config.read().unwrap().clone();
        let transcribers_rw = Arc::clone(&self.transcribers);
        let active_model_rw = Arc::clone(&self.active_model);
        // Передаём last_used в async-блок — обновим внутри lazy-reload ветки после
        // pool.insert. Sync-секция выше уже обновила last_used для active_name.
        let last_used_arc = Arc::clone(&self.last_used);
        let silence_timeout_secs = config.max_record_secs;
        let sample_rate = config.sample_rate;
        let auto_type = config.auto_type;
        let remove_fillers = config.remove_fillers;
        let vad_enabled = config.vad_enabled;
        let vad_silence_secs = config.vad_silence_secs;
        let debug = config.debug;
        // Effective gain зависит от **текущего активного** микрофона: если в БД есть
        // override для этого устройства — берём его, иначе глобальный mic_gain.
        // Это позволяет одной настройке работать со встроенным миком и наушниками.
        let device_name = audio::default_input_device_name().unwrap_or_default();
        let mic_gain = config.effective_gain(&device_name);
        let audio_level = Arc::clone(&self.audio_level);
        let handle = self.rt_handle.clone();
        let window_visible = Arc::clone(&self.window_visible);
        let history_db = Arc::clone(&self.history_db);

        self.rt_handle.spawn(async move {
            let mut busy_guard = is_busy.lock().await;
            let was_busy = *busy_guard;
            info!(
                trigger_call_id = call_id,
                was_busy, "trigger() async entered; deciding start vs stop"
            );

            if *busy_guard {
                // Останавливаем текущую запись
                let mut cmd_tx_guard = current_cmd_tx.lock().await;
                if let Some(tx) = cmd_tx_guard.take() {
                    info!(trigger_call_id = call_id, "trigger() → STOP recording");
                    let _ = tx.send(AudioCommand::Stop);
                } else {
                    info!(trigger_call_id = call_id, "Игнорирую триггер, идет обработка...");
                }
            } else {
                info!("Получен триггер для начала записи.");

                // Если конфиг изменился — пересоздаём транскрайбер (в фоне)
                if need_reload {
                    // Проверяем, может модель уже в пуле (предзагружена)
                    let target_name = config.transcriber_model_name();
                    let already_in_pool = transcribers_rw
                        .read()
                        .map(|pool| pool.get(&target_name).cloned())
                        .unwrap_or(None);

                    if let Some(t) = already_in_pool {
                        transcriber = t;
                        if let Ok(mut active) = active_model_rw.write() {
                            *active = target_name.clone();
                        }
                        info!("Модель '{}' взята из пула (была предзагружена)", target_name);
                    } else {
                        // Lazy-fallback: модели ещё нет в пуле (eager preload в save_config
                        // не успел или конфиг был изменён внешне). Эмитим тот же loading-event,
                        // что и preload_model — UI отработает одинаково.
                        let _ = event_tx.send(EngineEvent::ModelLoading(target_name.clone()));
                        let cfg = config.clone();
                        let transcribers_pool = Arc::clone(&transcribers_rw);
                        let reload_result = tokio::task::spawn_blocking(move || {
                            ArcanaEngine::create_transcriber(&cfg, &cfg.transcriber)
                        })
                        .await;

                        match reload_result {
                            Ok(Ok((name, t))) => {
                                transcriber = t.clone();
                                if let Ok(mut pool) = transcribers_pool.write() {
                                    pool.insert(name.clone(), t);
                                }
                                if let Ok(mut active) = active_model_rw.write() {
                                    *active = name.clone();
                                }
                                // LRU: засекаем загрузку модели — иначе sweeper мог бы её
                                // выгрузить уже на следующем тике, если предыдущая запись
                                // в last_used отсутствовала или была очень старой.
                                if let Ok(mut last) = last_used_arc.write() {
                                    last.insert(name.clone(), Instant::now());
                                }
                                info!("Модель '{}' загружена и добавлена в пул", name);
                                // Возвращаем UI в «готов»-состояние перед стартом записи.
                                let _ = event_tx.send(EngineEvent::ModelLoaded);
                            }
                            Ok(Err(e)) => {
                                tracing::error!("Не удалось пересоздать транскрайбер: {}", e);
                                let _ = event_tx.send(EngineEvent::Error(format!("Ошибка загрузки модели: {}", e)));
                                return;
                            }
                            Err(e) => {
                                tracing::error!("Ошибка загрузки модели: {:?}", e);
                                return;
                            }
                        }
                    } // else — модель не в пуле
                }

                // Проверяем микрофон перед записью (fail fast)
                let mic_check = tokio::task::spawn_blocking({
                    let sr = sample_rate;
                    move || audio::check_microphone(sr)
                })
                .await;

                let mic_err = mic_error_message(mic_check);
                if let Some(msg) = mic_err {
                    tracing::error!("{}", msg);
                    eprintln!("[Ошибка] {}", msg);
                    let is_visible = window_visible.load(Ordering::Relaxed);
                    if !is_visible {
                        let error_text = format!("[Ошибка микрофона] {}", msg);
                        let _ = crate::input::type_text(&error_text).await;
                    }
                    let _ = event_tx.send(EngineEvent::Error(msg));
                    return;
                }

                *busy_guard = true;
                *is_paused.lock().await = false;
                drop(busy_guard);

                let (cmd_tx, cmd_rx) = std_mpsc::channel();
                *current_cmd_tx.lock().await = Some(cmd_tx);

                let _ = event_tx.send(EngineEvent::RecordingStarted);

                // Вся фоновая запись/финализация — в run_record_session (owned-снимок).
                let session = RecordSession {
                    transcriber,
                    history_db,
                    event_tx: event_tx.clone(),
                    audio_level,
                    window_visible,
                    is_busy: Arc::clone(&is_busy),
                    is_paused: Arc::clone(&is_paused),
                    cmd_tx_cleanup: Arc::clone(&current_cmd_tx),
                    config,
                    sample_rate,
                    debug,
                    silence_timeout_secs,
                    vad_enabled,
                    vad_silence_secs,
                    mic_gain,
                    auto_type,
                    remove_fillers,
                };
                handle.spawn(run_record_session(session, cmd_rx));
            }
        });
    }

    /// Переключатель паузы: если записывает — приостановить/возобновить.
    pub fn pause(&self) {
        let is_busy = Arc::clone(&self.is_busy);
        let is_paused = Arc::clone(&self.is_paused);
        let current_cmd_tx = Arc::clone(&self.current_cmd_tx);
        let event_tx = self.event_tx.clone();

        self.rt_handle.spawn(async move {
            if !*is_busy.lock().await {
                info!("Игнорирую паузу, запись не идёт.");
                return;
            }

            let cmd_tx_guard = current_cmd_tx.lock().await;
            if let Some(tx) = cmd_tx_guard.as_ref() {
                let _ = tx.send(AudioCommand::TogglePause);
                let mut paused = is_paused.lock().await;
                *paused = !*paused;
                if *paused {
                    let _ = event_tx.send(EngineEvent::RecordingPaused);
                } else {
                    let _ = event_tx.send(EngineEvent::RecordingResumed);
                }
            }
        });
    }

    /// Прерывает текущую транскрибацию (только если активный движок поддерживает —
    /// сейчас Whisper через `whisper_full_params.abort_callback`). Для остальных
    /// движков no-op. Вызывается с UI thread'а в любой момент; если не транскрибация
    /// сейчас идёт — флаг просто будет проигнорирован при следующем transcribe().
    pub fn cancel_transcription(&self) -> bool {
        let active_name = self.active_model.read().unwrap().clone();
        let transcriber = self.transcribers.read().unwrap().get(&active_name).cloned();
        if let Some(t) = transcriber
            && t.supports_cancel()
        {
            t.cancel();
            return true;
        }
        false
    }

    /// Поддерживает ли активный движок отмену транскрибации.
    pub fn active_supports_cancel(&self) -> bool {
        let active_name = self.active_model.read().unwrap().clone();
        self.transcribers
            .read()
            .unwrap()
            .get(&active_name)
            .map(|t| t.supports_cancel())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Заглушка Transcriber для тестов LRU-логики (без реальной модели).
    struct DummyTranscriber;
    impl Transcriber for DummyTranscriber {
        fn transcribe(&self, _samples: &[i16], _sample_rate: u32) -> Result<String, ArcanaError> {
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

    #[test]
    fn test_mic_error_message() {
        assert!(mic_error_message(Ok(Ok(()))).is_none());
        assert!(mic_error_message(Ok(Err(ArcanaError::AudioDevice("нет".into())))).is_some());
    }
}
