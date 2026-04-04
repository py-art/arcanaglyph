// crates/arcanaglyph-core/src/audio.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tracing::info;
use vosk::Recognizer;

use crate::error::ArcanaError;

/// Команды управления записью
pub enum AudioCommand {
    /// Остановить запись и получить результат
    Stop,
    /// Приостановить/возобновить запись (переключатель)
    TogglePause,
}

/// Записывает аудио с микрофона и транскрибирует через Vosk.
/// Блокирующая функция — ждёт команд через `cmd_rx`.
/// Автоматически останавливается, если нет новых слов `silence_timeout_secs` секунд.
pub fn record_and_transcribe(
    cmd_rx: std_mpsc::Receiver<AudioCommand>,
    recognizer_arc: Arc<Mutex<Recognizer>>,
    sample_rate: u32,
    debug: bool,
    silence_timeout_secs: u64,
) -> Result<String, ArcanaError> {
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

    let recognizer_clone = Arc::clone(&recognizer_arc);
    let stream = device
        .build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let mut rec = recognizer_clone.lock().unwrap();
                if let Err(e) = rec.accept_waveform(data) {
                    tracing::error!("Ошибка при обработке аудиоданных: {:?}", e);
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

        if let Ok(mut rec) = recognizer_arc.lock() {
            let partial_text = rec.partial_result().partial.to_string();

            if !partial_text.is_empty() {
                let char_count = partial_text.chars().count();

                if char_count > segment_printed {
                    if debug {
                        let new_chars: String = partial_text.chars().skip(segment_printed).collect();
                        eprint!("{}", new_chars);
                        has_output = true;
                    }
                    segment_printed = char_count;
                }

                if partial_text.len() > max_partial_len {
                    max_partial_len = partial_text.len();
                    last_growth = Instant::now();
                }
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

    if paused {
        // Если были на паузе — нужно возобновить перед закрытием
        let _ = stream.play();
    }
    stream
        .pause()
        .map_err(|e| ArcanaError::AudioStream(format!("Не удалось остановить аудиопоток: {}", e)))?;

    info!("Запись завершена. Начинаю транскрибацию...");

    let mut recognizer_guard = recognizer_arc
        .lock()
        .map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;

    let final_result = recognizer_guard
        .final_result()
        .single()
        .ok_or_else(|| ArcanaError::Recognizer("Не удалось получить результат распознавания".into()))?;

    if debug {
        eprintln!("[Результат] {}", final_result.text);
    }
    info!("Финальный результат: {}", final_result.text);
    let result_text = final_result.text.to_string();
    recognizer_guard.reset();

    Ok(result_text)
}
