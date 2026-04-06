// crates/arcanaglyph-core/src/audio.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tracing::info;

use crate::engine::EngineEvent;
use crate::error::ArcanaError;
use crate::transcriber::Transcriber;

/// Команды управления записью
pub enum AudioCommand {
    /// Остановить запись и получить результат
    Stop,
    /// Приостановить/возобновить запись (переключатель)
    TogglePause,
}

/// Проверяет доступность микрофона перед началом записи (fail fast).
/// Открывает аудиопоток на 200 мс и проверяет, приходят ли данные.
pub fn check_microphone(sample_rate: u32) -> Result<(), ArcanaError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| ArcanaError::AudioDevice("Микрофон не найден. Подключите микрофон и проверьте настройки звука.".into()))?;

    let device_name = device.name().unwrap_or_else(|_| "неизвестно".into());
    info!("Микрофон: {}", device_name);

    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let got_audio = Arc::new(AtomicBool::new(false));
    let got_audio_clone = Arc::clone(&got_audio);

    let stream = device
        .build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                if data.iter().any(|&s| s != 0) {
                    got_audio_clone.store(true, Ordering::Relaxed);
                }
            },
            |err| tracing::error!("Ошибка проверки микрофона: {}", err),
            None,
        )
        .map_err(|e| ArcanaError::AudioDevice(format!(
            "Не удалось открыть микрофон '{}': {}. Проверьте настройки звука.", device_name, e
        )))?;

    stream.play().map_err(|e| ArcanaError::AudioDevice(format!(
        "Не удалось запустить микрофон '{}': {}", device_name, e
    )))?;

    // Ждём до 1 сек — PipeWire/ALSA может долго инициализировать поток
    for _ in 0..10 {
        thread::sleep(Duration::from_millis(100));
        if got_audio.load(Ordering::Relaxed) {
            break;
        }
    }

    drop(stream);

    if !got_audio.load(Ordering::Relaxed) {
        return Err(ArcanaError::AudioDevice(format!(
            "Микрофон '{}' не передаёт звук. Возможно, он отключён или выбрано неверное устройство.", device_name
        )));
    }

    info!("Микрофон '{}' работает", device_name);
    Ok(())
}

/// Результат записи и транскрибации
pub struct RecordResult {
    /// Распознанный текст
    pub text: String,
    /// Путь к аудиофайлу в кэше
    pub audio_path: String,
    /// Длительность записи (секунды)
    pub duration_secs: u32,
}

/// Записывает аудио с микрофона и транскрибирует через выбранный движок.
/// Блокирующая функция — ждёт команд через `cmd_rx`.
/// Автоматически останавливается, если нет новых слов `silence_timeout_secs` секунд.
#[allow(clippy::too_many_arguments)]
pub fn record_and_transcribe(
    cmd_rx: std_mpsc::Receiver<AudioCommand>,
    transcriber: &dyn Transcriber,
    sample_rate: u32,
    debug: bool,
    silence_timeout_secs: u64,
    audio_level: Arc<AtomicU32>,
    event_tx: tokio::sync::broadcast::Sender<EngineEvent>,
    audio_cache_dir: &std::path::Path,
) -> Result<RecordResult, ArcanaError> {
    let recording_start = std::time::Instant::now();
    info!("Начинаю запись...");

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| ArcanaError::AudioDevice("Нет доступного устройства ввода".into()))?;

    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let streaming = transcriber.supports_streaming();
    let level_clone = Arc::clone(&audio_level);

    // Счётчик ненулевых аудио-фреймов для детекции «мёртвого» микрофона
    let audio_frames_received = Arc::new(AtomicU32::new(0));
    let frames_clone = Arc::clone(&audio_frames_received);

    // Буфер для сбора всех сэмплов (нужен для Whisper и как fallback для Vosk)
    let all_samples: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
    let samples_clone = Arc::clone(&all_samples);

    // Для потокового режима (Vosk): используем канал для передачи данных из callback
    let (vosk_tx, vosk_rx) = if streaming {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<i16>>();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let stream = device
        .build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                // Считаем RMS (уровень громкости) и сохраняем в atomic (0-100)
                if !data.is_empty() {
                    let sum_sq: f64 = data.iter().map(|&s| (s as f64) * (s as f64)).sum();
                    let rms = (sum_sq / data.len() as f64).sqrt();
                    let level = ((rms / 3000.0).min(1.0) * 100.0) as u32;
                    level_clone.store(level, Ordering::Relaxed);
                    if rms > 10.0 {
                        frames_clone.fetch_add(1, Ordering::Relaxed);
                    }
                }

                // Собираем все сэмплы в буфер
                if let Ok(mut buf) = samples_clone.lock() {
                    buf.extend_from_slice(data);
                }

                // Для потокового режима (Vosk) — отправляем данные через канал
                if let Some(ref tx) = vosk_tx {
                    let _ = tx.send(data.to_vec());
                }
            },
            |err| tracing::error!("Ошибка в аудиопотоке: {}", err),
            None,
        )
        .map_err(|e| ArcanaError::AudioStream(format!("Не удалось создать аудиопоток: {}", e)))?;

    stream
        .play()
        .map_err(|e| ArcanaError::AudioStream(format!("Не удалось запустить аудиопоток: {}", e)))?;

    info!("Идет запись... (нажмите хоткей для останова или ждите таймаут)");

    let mut max_partial_len: usize = 0;
    let mut last_growth = Instant::now();
    let silence_timeout = Duration::from_secs(silence_timeout_secs);

    let mut segment_printed: usize = 0;
    let mut has_output = false;
    let mut paused = false;

    if debug {
        eprint!("[Запись] ");
    }

    loop {
        // Проверяем команды (неблокирующий)
        match cmd_rx.try_recv() {
            Ok(AudioCommand::Stop) => break,
            Ok(AudioCommand::TogglePause) => {
                if paused {
                    // Возобновляем
                    stream
                        .play()
                        .map_err(|e| ArcanaError::AudioStream(format!("Не удалось возобновить аудиопоток: {}", e)))?;
                    paused = false;
                    last_growth = Instant::now();
                    if debug {
                        if has_output {
                            eprintln!();
                        }
                        eprint!("[Запись] ");
                        has_output = false;
                    }
                } else {
                    // Приостанавливаем
                    stream
                        .pause()
                        .map_err(|e| ArcanaError::AudioStream(format!("Не удалось приостановить аудиопоток: {}", e)))?;
                    paused = true;
                    audio_level.store(0, Ordering::Relaxed);
                    if debug && has_output {
                        eprintln!();
                        has_output = false;
                    }
                    eprintln!("[Пауза]");
                }
            }
            Err(std_mpsc::TryRecvError::Empty) => {}
            Err(std_mpsc::TryRecvError::Disconnected) => break,
        }

        // Во время паузы не обрабатываем partial и не считаем тишину
        if paused {
            thread::sleep(Duration::from_millis(200));
            continue;
        }

        // Для потокового режима (Vosk): передаём данные из канала в транскрайбер
        if let Some(ref rx) = vosk_rx {
            while let Ok(data) = rx.try_recv() {
                if let Err(e) = transcriber.accept_waveform(&data) {
                    tracing::error!("Ошибка при обработке аудиоданных: {}", e);
                }
            }

            // Partial results — только для потокового режима в debug
            if debug {
                let partial_text = transcriber.partial_result();
                if !partial_text.is_empty() {
                    let char_count = partial_text.chars().count();
                    if char_count > segment_printed {
                        let new_chars: String = partial_text.chars().skip(segment_printed).collect();
                        eprint!("{}", new_chars);
                        has_output = true;
                        segment_printed = char_count;
                    }
                    if partial_text.len() > max_partial_len {
                        max_partial_len = partial_text.len();
                        last_growth = Instant::now();
                    }
                }
            }
        } else {
            // Для пакетного режима (Whisper): таймаут тишины по audio level
            let level = audio_level.load(Ordering::Relaxed);
            if level > 0 {
                last_growth = Instant::now();
            }
        }

        if last_growth.elapsed() >= silence_timeout {
            info!(
                "Запись останавливается по тишине ({}с без новых слов).",
                silence_timeout_secs
            );
            break;
        }

        thread::sleep(Duration::from_millis(200));
    }

    if debug && has_output {
        eprintln!();
    }
    eprintln!("[Запись остановлена]");
    audio_level.store(0, Ordering::Relaxed);

    if paused {
        let _ = stream.play();
    }
    stream
        .pause()
        .map_err(|e| ArcanaError::AudioStream(format!("Не удалось остановить аудиопоток: {}", e)))?;

    let recording_duration = recording_start.elapsed();
    info!("Запись завершена за {:.1}с. Начинаю транскрибацию...", recording_duration.as_secs_f64());

    // Проверяем, приходил ли звук с микрофона
    let frames = audio_frames_received.load(Ordering::Relaxed);
    if frames == 0 {
        tracing::warn!("За время записи не получено аудиоданных. Микрофон не подключён или выбрано неверное устройство.");
        eprintln!("[Ошибка] Микрофон не захватил звук. Проверьте подключение и настройки аудиоустройства.");
    }

    // Отправляем событие "Транскрибация..." для UI
    let _ = event_tx.send(EngineEvent::Transcribing);

    // Получаем собранные сэмплы
    let samples = all_samples.lock()
        .map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;

    let transcription_start = std::time::Instant::now();
    let result_text = transcriber.transcribe(&samples, sample_rate)?;
    let transcription_duration = transcription_start.elapsed();
    transcriber.reset();

    if debug {
        eprintln!("─────────────────────────────────────────");
        eprintln!("[Результат] {} ({:.1}с)", result_text, transcription_duration.as_secs_f64());
        eprintln!("─────────────────────────────────────────");
    }
    info!(
        "Финальный результат (запись: {:.1}с, транскрибация: {:.1}с): {}",
        recording_duration.as_secs_f64(),
        transcription_duration.as_secs_f64(),
        result_text
    );

    // Сохраняем аудио в кэш для повторной транскрибации другой моделью
    let timestamp = chrono::Utc::now().timestamp();
    let audio_filename = format!("{}.raw", timestamp);
    let audio_path = audio_cache_dir.join(&audio_filename);
    if let Err(e) = save_raw_audio(&audio_path, &samples) {
        tracing::warn!("Не удалось сохранить аудио в кэш: {}", e);
    }

    let duration_secs = recording_duration.as_secs() as u32;

    Ok(RecordResult {
        text: result_text,
        audio_path: audio_path.to_string_lossy().to_string(),
        duration_secs,
    })
}

/// Сохраняет сырые i16 сэмплы в файл
fn save_raw_audio(path: &std::path::Path, samples: &[i16]) -> Result<(), ArcanaError> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ArcanaError::Internal(format!("Не удалось создать директорию: {}", e)))?;
    }
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut file = std::fs::File::create(path)
        .map_err(|e| ArcanaError::Internal(format!("Не удалось создать файл: {}", e)))?;
    file.write_all(&bytes)
        .map_err(|e| ArcanaError::Internal(format!("Не удалось записать аудио: {}", e)))?;
    Ok(())
}
