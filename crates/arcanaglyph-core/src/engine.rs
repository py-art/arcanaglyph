// crates/arcanaglyph-core/src/engine.rs

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tracing::info;

use crate::audio::{self, AudioCommand};
use crate::config::{CoreConfig, TranscriberType};
use crate::error::ArcanaError;
use crate::history::HistoryDB;
use crate::transcriber::{Transcriber, VoskTranscriber, WhisperTranscriber};

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

impl ArcanaEngine {
    /// Создаёт новый экземпляр движка: загружает модель, инициализирует каналы.
    /// Должен вызываться из контекста Tokio runtime (сохраняет Handle для spawn).
    pub fn new(config: CoreConfig, window_visible: Arc<AtomicBool>) -> Result<Self, ArcanaError> {
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

        Ok(Self {
            config: std::sync::RwLock::new(config),
            transcribers: Arc::new(std::sync::RwLock::new(transcribers)),
            active_model: Arc::new(std::sync::RwLock::new(model_name)),
            config_changed: AtomicBool::new(false),
            is_busy: Arc::new(tokio::sync::Mutex::new(false)),
            is_paused: Arc::new(tokio::sync::Mutex::new(false)),
            current_cmd_tx: Arc::new(tokio::sync::Mutex::new(None)),
            event_tx,
            audio_level: Arc::new(AtomicU32::new(0)),
            rt_handle,
            window_visible,
            history_db,
        })
    }

    /// Создаёт транскрайбер по типу, возвращает (model_name, Arc<dyn Transcriber>)
    fn create_transcriber(config: &CoreConfig, t_type: &TranscriberType) -> Result<(String, Arc<dyn Transcriber>), ArcanaError> {
        match t_type {
            TranscriberType::Vosk => {
                let t = VoskTranscriber::new(&config.model_path, config.sample_rate as f32)?;
                let name = config.model_path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "vosk".to_string());
                Ok((name, Arc::new(t)))
            }
            TranscriberType::Whisper => {
                let t = WhisperTranscriber::new(&config.whisper_model_path)?;
                let name = config.whisper_model_path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "whisper".to_string());
                Ok((name, Arc::new(t)))
            }
        }
    }

    /// Предзагрузить модель в пул (вызывать из фонового потока)
    pub fn preload_model(&self, t_type: &TranscriberType) -> Result<String, ArcanaError> {
        let config = self.config.read().map_err(|e| ArcanaError::Internal(format!("RwLock: {}", e)))?;
        let (name, transcriber) = Self::create_transcriber(&config, t_type)?;

        let mut pool = self.transcribers.write().map_err(|e| ArcanaError::Internal(format!("RwLock: {}", e)))?;
        if !pool.contains_key(&name) {
            pool.insert(name.clone(), transcriber);
            info!("Модель '{}' предзагружена в пул", name);
        }
        Ok(name)
    }

    /// Список загруженных моделей
    pub fn loaded_models(&self) -> Vec<String> {
        self.transcribers.read().map(|pool| pool.keys().cloned().collect()).unwrap_or_default()
    }

    /// Имя активной модели
    pub fn active_model_name(&self) -> String {
        self.active_model.read().map(|m| m.clone()).unwrap_or_default()
    }

    /// Обновить конфигурацию — если модель уже в пуле, мгновенное переключение
    pub fn update_config(&self, new_config: CoreConfig) {
        // Определяем имя модели для нового конфига
        let new_model_name = match new_config.transcriber {
            TranscriberType::Vosk => new_config.model_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "vosk".to_string()),
            TranscriberType::Whisper => new_config.whisper_model_path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "whisper".to_string()),
        };

        // Если модель уже в пуле — мгновенное переключение
        let already_loaded = self.transcribers.read()
            .map(|pool| pool.contains_key(&new_model_name))
            .unwrap_or(false);

        if already_loaded {
            if let Ok(mut active) = self.active_model.write() {
                *active = new_model_name.clone();
            }
            info!("Модель '{}' уже загружена — мгновенное переключение", new_model_name);
        } else {
            self.config_changed.store(true, Ordering::Relaxed);
            info!("Модель '{}' не в пуле — загрузится при следующей записи", new_model_name);
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

    /// Переключатель записи: если не записывает — начать, если записывает — остановить.
    pub fn trigger(&self) {
        let is_busy = Arc::clone(&self.is_busy);
        let is_paused = Arc::clone(&self.is_paused);
        let current_cmd_tx = Arc::clone(&self.current_cmd_tx);
        let event_tx = self.event_tx.clone();
        let active_name = self.active_model.read().unwrap().clone();
        let mut transcriber = self.transcribers.read().unwrap()
            .get(&active_name).cloned()
            .unwrap_or_else(|| self.transcribers.read().unwrap().values().next().unwrap().clone());
        let need_reload = self.config_changed.swap(false, Ordering::Relaxed);
        let config = self.config.read().unwrap().clone();
        let transcribers_rw = Arc::clone(&self.transcribers);
        let active_model_rw = Arc::clone(&self.active_model);
        let silence_timeout_secs = config.max_record_secs;
        let sample_rate = config.sample_rate;
        let auto_type = config.auto_type;
        let debug = config.debug;
        let audio_level = Arc::clone(&self.audio_level);
        let handle = self.rt_handle.clone();
        let window_visible = Arc::clone(&self.window_visible);
        let history_db = Arc::clone(&self.history_db);

        self.rt_handle.spawn(async move {
            let mut busy_guard = is_busy.lock().await;

            if *busy_guard {
                // Останавливаем текущую запись
                let mut cmd_tx_guard = current_cmd_tx.lock().await;
                if let Some(tx) = cmd_tx_guard.take() {
                    let _ = tx.send(AudioCommand::Stop);
                } else {
                    info!("Игнорирую триггер, идет обработка...");
                }
            } else {
                info!("Получен триггер для начала записи.");

                // Если конфиг изменился — пересоздаём транскрайбер (в фоне)
                if need_reload {
                    let _ = event_tx.send(EngineEvent::Error("Загрузка модели...".to_string()));
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
                            info!("Модель '{}' загружена и добавлена в пул", name);
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
                }

                // Проверяем микрофон перед записью (fail fast)
                let mic_check = tokio::task::spawn_blocking({
                    let sr = sample_rate;
                    move || audio::check_microphone(sr)
                })
                .await;

                let mic_err = match mic_check {
                    Ok(Err(e)) => Some(e.to_string()),
                    Err(e) => Some(format!("Ошибка проверки микрофона: {:?}", e)),
                    Ok(Ok(())) => None,
                };
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

                let event_tx_clone = event_tx.clone();
                let cmd_tx_for_cleanup = Arc::clone(&current_cmd_tx);
                let is_busy_clone = Arc::clone(&is_busy);
                let is_paused_clone = Arc::clone(&is_paused);
                handle.spawn(async move {
                    let event_tx_audio = event_tx_clone.clone();
                    let transcriber_clone = transcriber;
                    let audio_cache = history_db.audio_cache_path().to_path_buf();
                    let result = tokio::task::spawn_blocking(move || {
                        audio::record_and_transcribe(
                            cmd_rx,
                            transcriber_clone.as_ref(),
                            sample_rate,
                            debug,
                            silence_timeout_secs,
                            audio_level,
                            event_tx_audio,
                            &audio_cache,
                        )
                    })
                    .await;

                    match result {
                        Ok(Ok(rec)) => {
                            if rec.text.is_empty() {
                                tracing::warn!("Распознавание вернуло пустой результат. Проверьте микрофон.");
                                let _ = event_tx_clone.send(EngineEvent::Error(
                                    "Микрофон не захватил речь. Проверьте, что микрофон подключён и выбран как устройство по умолчанию.".to_string(),
                                ));
                            } else {
                                // Сохраняем в историю
                                let model_name = config.transcriber_model_name();
                                let transcriber_type_str = config.transcriber_type_str();
                                if let Err(e) = (|| -> Result<(), crate::error::ArcanaError> {
                                    let rec_id = history_db.add_recording(&rec.audio_path, rec.duration_secs)?;
                                    history_db.add_transcription(rec_id, &rec.text, &model_name, &transcriber_type_str)?;
                                    Ok(())
                                })() {
                                    tracing::warn!("Не удалось сохранить в историю: {}", e);
                                }

                                let is_visible = window_visible.load(Ordering::Relaxed);
                                // Вставляем текст только когда окно скрыто (в трее)
                                if auto_type && !is_visible {
                                    eprintln!("[Вставка] в активное окно...");
                                    if let Err(e) = crate::input::type_text(&rec.text).await {
                                        tracing::error!("Не удалось вставить текст: {}", e);
                                    }
                                }
                                let _ = event_tx_clone.send(EngineEvent::TranscriptionResult(rec.text));
                                // Если окно видимо — запрашиваем фокус для вывода на передний план
                                if is_visible {
                                    let _ = event_tx_clone.send(EngineEvent::RequestFocus);
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            tracing::error!("Ошибка транскрибации: {}", e);
                            let _ = event_tx_clone.send(EngineEvent::Error(format!("Ошибка транскрибации: {}", e)));
                        }
                        Err(e) => {
                            tracing::error!("Задача записи завершилась с ошибкой: {:?}", e);
                            let _ = event_tx_clone.send(EngineEvent::Error(format!("Ошибка записи: {:?}", e)));
                        }
                    }

                    info!("Обработка завершена. Система готова к новой записи.");
                    let _ = event_tx_clone.send(EngineEvent::FinishedProcessing);

                    *is_busy_clone.lock().await = false;
                    *is_paused_clone.lock().await = false;
                    *cmd_tx_for_cleanup.lock().await = None;
                });
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
}
