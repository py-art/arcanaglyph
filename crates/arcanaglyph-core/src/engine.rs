// crates/arcanaglyph-core/src/engine.rs

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tracing::info;

use crate::audio::{self, AudioCommand};
use crate::config::{CoreConfig, TranscriberType};
use crate::error::ArcanaError;
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
    /// Ошибка, которую нужно показать пользователю
    Error(String),
}

/// Основной движок ArcanaGlyph: управляет записью, распознаванием и рассылкой событий
pub struct ArcanaEngine {
    config: CoreConfig,
    transcriber: Arc<dyn Transcriber>,
    is_busy: Arc<tokio::sync::Mutex<bool>>,
    is_paused: Arc<tokio::sync::Mutex<bool>>,
    current_cmd_tx: Arc<tokio::sync::Mutex<Option<std_mpsc::Sender<AudioCommand>>>>,
    event_tx: broadcast::Sender<EngineEvent>,
    audio_level: Arc<AtomicU32>,
    rt_handle: Handle,
    /// Флаг видимости окна: true — окно видимо, false — свёрнуто в трей
    window_visible: Arc<AtomicBool>,
}

impl ArcanaEngine {
    /// Создаёт новый экземпляр движка: загружает модель, инициализирует каналы.
    /// Должен вызываться из контекста Tokio runtime (сохраняет Handle для spawn).
    pub fn new(config: CoreConfig, window_visible: Arc<AtomicBool>) -> Result<Self, ArcanaError> {
        let transcriber: Arc<dyn Transcriber> = match config.transcriber {
            TranscriberType::Vosk => {
                Arc::new(VoskTranscriber::new(&config.model_path, config.sample_rate as f32)?)
            }
            TranscriberType::Whisper => {
                Arc::new(WhisperTranscriber::new(&config.whisper_model_path)?)
            }
        };

        let (event_tx, _) = broadcast::channel::<EngineEvent>(32);

        let rt_handle = Handle::try_current().map_err(|_| {
            ArcanaError::Internal("ArcanaEngine::new() должен вызываться из контекста Tokio runtime".into())
        })?;

        Ok(Self {
            config,
            transcriber,
            is_busy: Arc::new(tokio::sync::Mutex::new(false)),
            is_paused: Arc::new(tokio::sync::Mutex::new(false)),
            current_cmd_tx: Arc::new(tokio::sync::Mutex::new(None)),
            event_tx,
            audio_level: Arc::new(AtomicU32::new(0)),
            rt_handle,
            window_visible,
        })
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
        let transcriber = Arc::clone(&self.transcriber);
        let silence_timeout_secs = self.config.max_record_secs;
        let sample_rate = self.config.sample_rate;
        let auto_type = self.config.auto_type;
        let debug = self.config.debug;
        let audio_level = Arc::clone(&self.audio_level);
        let handle = self.rt_handle.clone();
        let window_visible = Arc::clone(&self.window_visible);

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
                // Проверяем микрофон перед записью (fail fast)
                info!("Получен триггер для начала записи.");
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
                    let result = tokio::task::spawn_blocking(move || {
                        audio::record_and_transcribe(
                            cmd_rx,
                            transcriber.as_ref(),
                            sample_rate,
                            debug,
                            silence_timeout_secs,
                            audio_level,
                            event_tx_audio,
                        )
                    })
                    .await;

                    match result {
                        Ok(Ok(text)) => {
                            if text.is_empty() {
                                tracing::warn!("Распознавание вернуло пустой результат. Проверьте микрофон.");
                                let _ = event_tx_clone.send(EngineEvent::Error(
                                    "Микрофон не захватил речь. Проверьте, что микрофон подключён и выбран как устройство по умолчанию.".to_string(),
                                ));
                            } else {
                                let is_visible = window_visible.load(Ordering::Relaxed);
                                // Вставляем текст только когда окно скрыто (в трее)
                                if auto_type && !is_visible {
                                    eprintln!("[Вставка] в активное окно...");
                                    if let Err(e) = crate::input::type_text(&text).await {
                                        tracing::error!("Не удалось вставить текст: {}", e);
                                    }
                                }
                                let _ = event_tx_clone.send(EngineEvent::TranscriptionResult(text));
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
