// crates/arcanaglyph-core/src/engine/mod.rs

mod lru;
mod record_session;

use record_session::{RecordSession, resolve_reloaded_transcriber, run_record_session};

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
use crate::history::HistoryDB;
// Конкретные транскрайберы (Vosk/Whisper/GigaAm/Qwen3Asr) больше не импортируются
// здесь напрямую — их конструирует единая фабрика `transcriber::build_transcriber`.
use crate::transcriber::Transcriber;

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

/// Дебаунс double-trigger: `true` → этот вызов нужно проглотить (повтор в пределах
/// 250мс от предыдущего). На GNOME одна клавиша приходит и через Tauri global-
/// shortcut, и через GNOME→`arcanaglyph --trigger`→Unix-сокет — два `trigger()` за миллисекунды дают
/// start+stop («визуал пляшет»). Человек физически не успевает нажать старт+стоп
/// за это время, так что окно безопасно. Состояние — в process-global `static`.
fn trigger_debounced(now: Instant) -> bool {
    static LAST_TRIGGER: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let mut last = LAST_TRIGGER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(prev) = *last
        && now.duration_since(prev) < Duration::from_millis(250)
    {
        return true;
    }
    *last = Some(now);
    false
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
    /// Конструирование делегировано единой фабрике `transcriber::build_transcriber`;
    /// здесь добавляется только инициализация ORT-логирования (зависит от линковки).
    fn create_transcriber(
        config: &CoreConfig,
        t_type: &TranscriberType,
    ) -> Result<(String, Arc<dyn Transcriber>), ArcanaError> {
        // init_ort_logging() ВРЕМЕННО только для feature `gigaam` (статически
        // линкованный ORT). Для `gigaam-system-ort` (load-dynamic) вызов
        // Environment::current() до dlopen сессии может зависнуть — пропускаем.
        #[cfg(feature = "gigaam")]
        if matches!(t_type, TranscriberType::GigaAm) {
            Self::init_ort_logging();
        }
        #[cfg(feature = "qwen3asr")]
        if matches!(t_type, TranscriberType::Qwen3Asr) {
            Self::init_ort_logging();
        }
        let t = crate::transcriber::build_transcriber(config, t_type)?;
        Ok((config.model_name_for(t_type), Arc::from(t)))
    }

    /// Предзагрузить модель в пул (вызывать из фонового потока).
    /// Пропускает загрузку если модель уже в пуле.
    pub fn preload_model(&self, t_type: &TranscriberType) -> Result<String, ArcanaError> {
        let config = self
            .config
            .read()
            .map_err(|e| ArcanaError::Internal(format!("RwLock: {}", e)))?;

        // Определяем имя модели без загрузки — чтобы проверить пул
        let expected_name = config.model_name_for(t_type);

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
        let new_model_name = new_config.model_name_for(&new_config.transcriber);

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

        // Дебаунс double-trigger (см. trigger_debounced): глушим повтор в пределах
        // 250мс, иначе одна клавиша на GNOME даёт start+stop и «визуал пляшет».
        if trigger_debounced(Instant::now()) {
            info!(trigger_call_id = call_id, "trigger() debounced (double-trigger guard)");
            return;
        }

        let is_busy = Arc::clone(&self.is_busy);
        let is_paused = Arc::clone(&self.is_paused);
        let current_cmd_tx = Arc::clone(&self.current_cmd_tx);
        let event_tx = self.event_tx.clone();
        let active_name = self.active_model.read().unwrap_or_else(|e| e.into_inner()).clone();
        // LRU: обновляем last_used для активной модели — каждое нажатие Ctrl+Ё
        // отодвигает время выгрузки. Без этого активно используемая модель могла
        // бы попасть под выгрузку, если пользователь делает паузы > TTL между записями.
        if let Ok(mut last) = self.last_used.write() {
            last.insert(active_name.clone(), Instant::now());
        }
        // Берём активный транскрайбер; если его нет в пуле — fallback на первый
        // доступный. Пустой пул (модель ещё не загружена) больше НЕ паникует —
        // эмитим Error и выходим, UI покажет сообщение вместо краша приложения.
        let transcriber = {
            let pool = self.transcribers.read().unwrap_or_else(|e| e.into_inner());
            match pool
                .get(&active_name)
                .cloned()
                .or_else(|| pool.values().next().cloned())
            {
                Some(t) => t,
                None => {
                    tracing::error!("Пул транскрайберов пуст — модель ещё не загружена");
                    let _ = event_tx.send(EngineEvent::Error(
                        "Модель ещё не загружена, подождите завершения загрузки".to_string(),
                    ));
                    return;
                }
            }
        };
        let need_reload = self.config_changed.swap(false, Ordering::Relaxed);
        let config = self.config.read().unwrap_or_else(|e| e.into_inner()).clone();
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

            // Уже записываем → это запрос на остановку. Шлём Stop и выходим.
            if *busy_guard {
                let mut cmd_tx_guard = current_cmd_tx.lock().await;
                if let Some(tx) = cmd_tx_guard.take() {
                    info!(trigger_call_id = call_id, "trigger() → STOP recording");
                    let _ = tx.send(AudioCommand::Stop);
                } else {
                    info!(trigger_call_id = call_id, "Игнорирую триггер, идет обработка...");
                }
                return;
            }

            info!("Получен триггер для начала записи.");

            // Если конфиг изменился — разрешаем/пересоздаём транскрайбер (в фоне).
            let Some(transcriber) = resolve_reloaded_transcriber(
                need_reload,
                transcriber,
                &config,
                &transcribers_rw,
                &active_model_rw,
                &last_used_arc,
                &event_tx,
            )
            .await
            else {
                return;
            };

            // Микрофон больше НЕ проверяем отдельным probe-потоком: запись стартует
            // сразу (плашка появляется по нажатию, а не после первого слова), а
            // мёртвый/молчащий микрофон ловится грейс-окном живости внутри
            // `record_and_transcribe` на том же потоке — без двойного cold-start и
            // потери начала фразы. Ошибка приходит в UI через `EngineEvent::Error`.
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
        let active_name = self.active_model.read().unwrap_or_else(|e| e.into_inner()).clone();
        let transcriber = self
            .transcribers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&active_name)
            .cloned();
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
        let active_name = self.active_model.read().unwrap_or_else(|e| e.into_inner()).clone();
        self.transcribers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&active_name)
            .map(|t| t.supports_cancel())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Заглушка Transcriber: без реальной модели, без поддержки отмены.
    struct DummyTranscriber;
    impl Transcriber for DummyTranscriber {
        fn transcribe(&self, _samples: &[i16], _sample_rate: u32) -> Result<String, ArcanaError> {
            Ok(String::new())
        }
        fn supports_streaming(&self) -> bool {
            false
        }
    }

    /// Временная БД истории под тест.
    fn temp_history_db(test_id: &str) -> Arc<HistoryDB> {
        let dir = std::env::temp_dir().join(format!("arcanaglyph_engine_test_{}_{}", test_id, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Arc::new(HistoryDB::new(&dir.join("history.db"), dir.join("audio")).expect("history db"))
    }

    /// Собирает `ArcanaEngine` для тестов: инъекция dummy-транскрайберов под
    /// заданными именами + временная БД, без загрузки реальной модели и без
    /// sweeper'а. tests-mod — дочерний модуль, поэтому имеет доступ к приватным
    /// полям (обходим `new()`, который требует ONNX-файлы и Tokio-runtime spawn).
    fn make_engine(config: CoreConfig, active: &str, model_names: &[&str], test_id: &str) -> ArcanaEngine {
        let mut transcribers: HashMap<String, Arc<dyn Transcriber>> = HashMap::new();
        let mut last_used = HashMap::new();
        for name in model_names {
            transcribers.insert((*name).to_string(), Arc::new(DummyTranscriber));
            last_used.insert((*name).to_string(), Instant::now());
        }
        let (event_tx, _) = broadcast::channel::<EngineEvent>(32);
        ArcanaEngine {
            config: std::sync::RwLock::new(config),
            transcribers: Arc::new(std::sync::RwLock::new(transcribers)),
            active_model: Arc::new(std::sync::RwLock::new(active.to_string())),
            last_used: Arc::new(std::sync::RwLock::new(last_used)),
            config_changed: AtomicBool::new(false),
            is_busy: Arc::new(tokio::sync::Mutex::new(false)),
            is_paused: Arc::new(tokio::sync::Mutex::new(false)),
            current_cmd_tx: Arc::new(tokio::sync::Mutex::new(None)),
            event_tx,
            audio_level: Arc::new(AtomicU32::new(0)),
            rt_handle: Handle::current(),
            window_visible: Arc::new(AtomicBool::new(false)),
            history_db: temp_history_db(test_id),
        }
    }

    #[tokio::test]
    async fn test_getters_reflect_injected_state() {
        // `CoreConfig::default().transcriber` = Vosk, поэтому выставляем GigaAm
        // явно — чтобы активная модель и тип в конфиге согласовались.
        let config = CoreConfig {
            transcriber: TranscriberType::GigaAm,
            ..CoreConfig::default()
        };
        let name = config.model_name_for(&TranscriberType::GigaAm);
        let engine = make_engine(config.clone(), &name, &[&name], "getters");

        assert_eq!(engine.active_model_name(), name);
        assert_eq!(engine.active_transcriber_type(), TranscriberType::GigaAm);
        assert!(engine.loaded_models().contains(&name));
        assert!(engine.loaded_models_idle_seconds().contains_key(&name));
        assert_eq!(engine.get_audio_level(), 0);
        assert_eq!(engine.show_widget(), config.show_widget);
    }

    #[tokio::test]
    async fn test_update_config_switches_to_already_loaded_model() {
        let config = CoreConfig::default();
        let giga = config.model_name_for(&TranscriberType::GigaAm);
        let whisper = config.model_name_for(&TranscriberType::Whisper);
        // Обе модели уже в пуле, активна gigaam.
        let engine = make_engine(config.clone(), &giga, &[&giga, &whisper], "switch_loaded");

        let mut new_config = config.clone();
        new_config.transcriber = TranscriberType::Whisper;
        engine.update_config(new_config);

        // Модель уже в пуле → мгновенное переключение активной.
        assert_eq!(engine.active_model_name(), whisper);
        assert_eq!(engine.active_transcriber_type(), TranscriberType::Whisper);
    }

    #[tokio::test]
    async fn test_update_config_defers_load_when_model_not_in_pool() {
        let config = CoreConfig::default();
        let giga = config.model_name_for(&TranscriberType::GigaAm);
        // Только gigaam в пуле.
        let engine = make_engine(config.clone(), &giga, &[&giga], "defer_load");

        let mut new_config = config.clone();
        new_config.transcriber = TranscriberType::Whisper;
        engine.update_config(new_config);

        // Модели нет в пуле → активная НЕ меняется (загрузится при следующей
        // записи), но тип в конфиге обновлён.
        assert_eq!(engine.active_model_name(), giga);
        assert_eq!(engine.active_transcriber_type(), TranscriberType::Whisper);
    }

    #[tokio::test]
    async fn test_cancel_unsupported_for_dummy_engine() {
        let config = CoreConfig::default();
        let name = config.model_name_for(&TranscriberType::GigaAm);
        let engine = make_engine(config, &name, &[&name], "cancel");

        // DummyTranscriber не поддерживает отмену → оба метода возвращают false.
        assert!(!engine.active_supports_cancel());
        assert!(!engine.cancel_transcription());
    }
}
