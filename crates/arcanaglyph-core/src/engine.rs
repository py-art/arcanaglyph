// crates/arcanaglyph-core/src/engine.rs

use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tracing::info;
use vosk::{LogLevel, Model, Recognizer};

use crate::audio;
use crate::config::CoreConfig;
use crate::error::ArcanaError;

/// События движка, рассылаемые подписчикам
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Запись началась
    RecordingStarted,
    /// Результат транскрибации
    TranscriptionResult(String),
    /// Обработка завершена, система готова к новой записи
    FinishedProcessing,
}

/// Основной движок ArcanaGlyph: управляет записью, распознаванием и рассылкой событий
pub struct ArcanaEngine {
    config: CoreConfig,
    recognizer: Arc<Mutex<Recognizer>>,
    is_busy: Arc<tokio::sync::Mutex<bool>>,
    current_stop_tx: Arc<tokio::sync::Mutex<Option<std_mpsc::Sender<()>>>>,
    event_tx: broadcast::Sender<EngineEvent>,
    rt_handle: Handle,
}

impl ArcanaEngine {
    /// Создаёт новый экземпляр движка: загружает модель Vosk, инициализирует каналы.
    /// Должен вызываться из контекста Tokio runtime (сохраняет Handle для spawn).
    pub fn new(config: CoreConfig) -> Result<Self, ArcanaError> {
        vosk::set_log_level(LogLevel::Error);

        info!("Загрузка модели из: {:?}", config.model_path);
        let model_path_str = config
            .model_path
            .to_str()
            .ok_or_else(|| ArcanaError::ModelLoad("Невалидный путь к модели (не UTF-8)".into()))?;

        let model = Model::new(model_path_str)
            .ok_or_else(|| ArcanaError::ModelLoad(format!("Не удалось загрузить модель из: {}", model_path_str)))?;
        info!("Модель успешно загружена.");

        let recognizer = Recognizer::new(&model, config.sample_rate as f32)
            .ok_or_else(|| ArcanaError::Recognizer("Не удалось создать распознаватель".into()))?;

        let (event_tx, _) = broadcast::channel::<EngineEvent>(32);

        // Сохраняем Handle к текущему Tokio runtime, чтобы trigger() работал из любого потока
        let rt_handle = Handle::try_current().map_err(|_| {
            ArcanaError::Internal("ArcanaEngine::new() должен вызываться из контекста Tokio runtime".into())
        })?;

        Ok(Self {
            config,
            recognizer: Arc::new(Mutex::new(recognizer)),
            is_busy: Arc::new(tokio::sync::Mutex::new(false)),
            current_stop_tx: Arc::new(tokio::sync::Mutex::new(None)),
            event_tx,
            rt_handle,
        })
    }

    /// Подписаться на события движка
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.event_tx.subscribe()
    }

    /// Проверить, идёт ли сейчас запись
    pub async fn is_recording(&self) -> bool {
        *self.is_busy.lock().await
    }

    /// Переключатель записи: если не записывает — начать, если записывает — остановить.
    /// Безопасно вызывать из любого потока (tray handler, hotkey handler, Tauri command).
    pub fn trigger(&self) {
        let is_busy = Arc::clone(&self.is_busy);
        let current_stop_tx = Arc::clone(&self.current_stop_tx);
        let event_tx = self.event_tx.clone();
        let recognizer = Arc::clone(&self.recognizer);
        let silence_timeout_secs = self.config.max_record_secs;
        let sample_rate = self.config.sample_rate;
        let auto_type = self.config.auto_type;
        let debug = self.config.debug;
        let handle = self.rt_handle.clone();

        self.rt_handle.spawn(async move {
            let mut busy_guard = is_busy.lock().await;

            if *busy_guard {
                // Останавливаем текущую запись
                let mut stop_tx_guard = current_stop_tx.lock().await;
                if let Some(tx) = stop_tx_guard.take() {
                    // Не логируем здесь — audio.rs сам напечатает [Запись остановлена]
                    let _ = tx.send(());
                } else {
                    info!("Игнорирую триггер, идет обработка...");
                }
            } else {
                // Начинаем новую запись
                info!("Получен триггер для начала записи.");
                *busy_guard = true;
                drop(busy_guard); // Освобождаем lock перед долгими операциями

                let (local_stop_tx, local_stop_rx) = std_mpsc::channel();
                *current_stop_tx.lock().await = Some(local_stop_tx);

                let _ = event_tx.send(EngineEvent::RecordingStarted);

                // Запись и транскрибация в блокирующей задаче
                // (автоостановка по тишине реализована внутри audio::record_and_transcribe)
                let event_tx_clone = event_tx.clone();
                let stop_tx_for_recorder = Arc::clone(&current_stop_tx);
                let is_busy_clone = Arc::clone(&is_busy);
                handle.spawn(async move {
                    let recognizer_clone = Arc::clone(&recognizer);
                    let result = tokio::task::spawn_blocking(move || {
                        audio::record_and_transcribe(
                            local_stop_rx,
                            recognizer_clone,
                            sample_rate,
                            debug,
                            silence_timeout_secs,
                        )
                    })
                    .await;

                    match result {
                        Ok(Ok(text)) => {
                            // Автоматическая вставка текста в активное окно
                            if auto_type
                                && !text.is_empty()
                                && let Err(e) = crate::input::type_text(&text)
                            {
                                tracing::error!("Не удалось вставить текст: {}", e);
                            }
                            let _ = event_tx_clone.send(EngineEvent::TranscriptionResult(text));
                        }
                        Ok(Err(e)) => {
                            tracing::error!("Ошибка транскрибации: {}", e);
                        }
                        Err(e) => {
                            tracing::error!("Задача записи завершилась с ошибкой: {:?}", e);
                        }
                    }

                    info!("Обработка завершена. Система готова к новой записи.");
                    let _ = event_tx_clone.send(EngineEvent::FinishedProcessing);

                    *is_busy_clone.lock().await = false;
                    *stop_tx_for_recorder.lock().await = None;
                });
            }
        });
    }
}
