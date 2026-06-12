// crates/arcanaglyph-core/src/audio.rs

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tracing::info;

use crate::engine::EngineEvent;
use crate::error::ArcanaError;
use crate::transcriber::Transcriber;

/// Возвращает user-friendly имя текущего default-микрофона.
///
/// На Linux с PipeWire (через wpctl) — `node.description` от @DEFAULT_AUDIO_SOURCE@.
/// Это даёт реальные имена ("Built-in Audio Аналоговый стерео", "Anker SoundCore
/// Headset") которые **меняются** при подключении/отключении наушников.
/// cpal на PipeWire всегда показывает только "default" — поэтому per-device
/// usage gain на cpal name не работает (один и тот же ключ для всех мик).
///
/// Fallback (если wpctl нет / не PipeWire / не Linux) — cpal `Device::name()`.
pub fn default_input_device_name() -> Option<String> {
    // Linux + wpctl: реальное имя через PipeWire
    #[cfg(target_os = "linux")]
    if let Some(name) = wpctl_default_source_description() {
        return Some(name);
    }
    // Fallback на cpal
    let host = cpal::default_host();
    host.default_input_device().and_then(|d| d.name().ok())
}

/// Парсит вывод `wpctl inspect @DEFAULT_AUDIO_SOURCE@` и возвращает `node.description`.
/// Возвращает None если wpctl не установлен / нет PipeWire / не нашлось description.
#[cfg(target_os = "linux")]
fn wpctl_default_source_description() -> Option<String> {
    let output = std::process::Command::new("wpctl")
        .args(["inspect", "@DEFAULT_AUDIO_SOURCE@"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        // Формат: `  * node.description = "Built-in Audio Аналоговый стерео"`
        // Также бывает без `*` (если не помечено): `  node.description = "..."`
        let trimmed = line.trim_start_matches(['*', ' ', '\t']);
        if let Some(rest) = trimmed.strip_prefix("node.description") {
            // rest = ` = "Built-in Audio Аналоговый стерео"`
            let value = rest.trim().trim_start_matches('=').trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

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
    let device = host.default_input_device().ok_or_else(|| {
        ArcanaError::AudioDevice("Микрофон не найден. Подключите микрофон и проверьте настройки звука.".into())
    })?;

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
        .map_err(|e| {
            ArcanaError::AudioDevice(format!(
                "Не удалось открыть микрофон '{}': {}. Проверьте настройки звука.",
                device_name, e
            ))
        })?;

    stream
        .play()
        .map_err(|e| ArcanaError::AudioDevice(format!("Не удалось запустить микрофон '{}': {}", device_name, e)))?;

    // Ждём до 1 сек — PipeWire/ALSA может долго инициализировать поток
    for _ in 0..10 {
        thread::sleep(Duration::from_millis(100));
        if got_audio.load(Ordering::Relaxed) {
            break;
        }
    }

    drop(stream);

    if !got_audio.load(Ordering::Relaxed) {
        // Самая частая причина на голом ALSA (без PulseAudio/PipeWire) — Capture switch
        // в микшере замьючен. Подсказываем команду размьюта прямо в сообщении.
        return Err(ArcanaError::AudioDevice(format!(
            "Микрофон '{}' не передаёт звук (только тишина за 1с). \
             Возможные причины: микрофон замьючен в ALSA-микшере, выбрано неверное устройство, \
             или микрофон физически отключён. Проверьте: `amixer -c 0 sget Capture` — если стоит \
             [off], размьютьте: `amixer -c 0 sset Capture cap` и поднимите усиление: \
             `amixer -c 0 sset 'Internal Mic Boost' 100%`.",
            device_name
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

/// Неизменяемые настройки сессии записи (конфиг записи + путь кэша).
/// Owned (без lifetime) — удобно move'ить в `spawn_blocking` вызывающего.
pub struct RecordParams {
    pub sample_rate: u32,
    pub debug: bool,
    pub silence_timeout_secs: u64,
    pub vad_enabled: bool,
    pub vad_silence_secs: u64,
    pub mic_gain: f32,
    pub audio_cache_dir: std::path::PathBuf,
}

/// Каналы и shared-состояние сессии записи (связь с движком/UI).
pub struct RecordChannels {
    pub cmd_rx: std_mpsc::Receiver<AudioCommand>,
    pub audio_level: Arc<AtomicU32>,
    pub event_tx: tokio::sync::broadcast::Sender<EngineEvent>,
}

/// Живой cpal-stream + shared-буферы, заполняемые из аудио-callback.
/// `cpal::Stream` не `Send` — живёт целиком внутри `record_and_transcribe`
/// (которая исполняется в blocking-потоке), не переживает `.await`.
struct CaptureHandles {
    stream: cpal::Stream,
    all_samples: Arc<Mutex<Vec<i16>>>,
    audio_frames_received: Arc<AtomicU32>,
    vosk_rx: Option<std::sync::mpsc::Receiver<Vec<i16>>>,
}

/// Применяет программное усиление микрофона с saturation на ±32767.
/// `gain <= 0` или ≈1.0 → `Cow::Borrowed(raw)` (без аллокации). Чистая функция.
fn apply_mic_gain(raw: &[i16], gain: f32) -> Cow<'_, [i16]> {
    if gain > 0.0 && (gain - 1.0).abs() > f32::EPSILON {
        Cow::Owned(
            raw.iter()
                .map(|&s| ((s as f32) * gain).clamp(i16::MIN as f32, i16::MAX as f32) as i16)
                .collect(),
        )
    } else {
        Cow::Borrowed(raw)
    }
}

/// RMS → уровень громкости 0..100 и флаг «живого» сигнала (rms > 10).
/// Пустой вход → `(0, false)`. Чистая функция.
fn compute_rms_level(data: &[i16]) -> (u32, bool) {
    if data.is_empty() {
        return (0, false);
    }
    let sum_sq: f64 = data.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum_sq / data.len() as f64).sqrt();
    let level = ((rms / 3000.0).min(1.0) * 100.0) as u32;
    (level, rms > 10.0)
}

/// Причина авто-остановки записи по тишине.
enum StopReason {
    /// VAD: речь была и тишина длилась дольше `vad_silence_secs`.
    Vad,
    /// Общий таймаут безопасности (`max_record_secs`).
    Timeout,
}

/// Чистое решение об авто-остановке. VAD проверяется первым (как в оригинале).
/// Длительности передаются явно (`elapsed()` снаружи) — функция тестируема.
fn should_stop_on_silence(
    vad_enabled: bool,
    speech_detected: bool,
    since_last_speech: Duration,
    vad_timeout: Duration,
    since_last_growth: Duration,
    silence_timeout: Duration,
) -> Option<StopReason> {
    if vad_enabled && speech_detected && since_last_speech >= vad_timeout {
        return Some(StopReason::Vad);
    }
    if since_last_growth >= silence_timeout {
        return Some(StopReason::Timeout);
    }
    None
}

/// Создаёт и запускает cpal input-stream: применяет mic_gain, считает RMS-уровень,
/// собирает сэмплы в буфер и (для streaming-движков) шлёт их в vosk-канал.
fn build_capture_stream(
    sample_rate: u32,
    mic_gain: f32,
    streaming: bool,
    audio_level: Arc<AtomicU32>,
) -> Result<CaptureHandles, ArcanaError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| ArcanaError::AudioDevice("Нет доступного устройства ввода".into()))?;

    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

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

    // Применяем mic_gain если задано (>0 и не ровно 1.0). Saturation на ±32767.
    // Делаем ОДНИМ buffer-clone в callback, всё последующее (RMS, all_samples,
    // vosk_tx) использует уже усиленные данные.
    if mic_gain > 0.0 && (mic_gain - 1.0).abs() > f32::EPSILON {
        info!("Программное усиление микрофона: x{:.2}", mic_gain);
    }
    let stream = device
        .build_input_stream(
            &config,
            move |raw: &[i16], _: &cpal::InputCallbackInfo| {
                // Локальный буфер с применённым gain (или borrow от raw на gain=1.0)
                let data = apply_mic_gain(raw, mic_gain);

                // Считаем RMS (уровень громкости) и сохраняем в atomic (0-100)
                if !data.is_empty() {
                    let (level, voiced) = compute_rms_level(&data);
                    level_clone.store(level, Ordering::Relaxed);
                    if voiced {
                        frames_clone.fetch_add(1, Ordering::Relaxed);
                    }
                }

                // Собираем все сэмплы в буфер
                if let Ok(mut buf) = samples_clone.lock() {
                    buf.extend_from_slice(&data);
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

    Ok(CaptureHandles {
        stream,
        all_samples,
        audio_frames_received,
        vosk_rx,
    })
}

/// Изменяемое состояние цикла записи: флаг паузы, прогресс debug-вывода и тайминги
/// для VAD/таймаута. Вынесено из локалов `record_and_transcribe`, чтобы шаги цикла
/// (`handle_command`) мутировали его по `&mut`.
struct LoopState {
    paused: bool,
    has_output: bool,
    segment_printed: usize,
    max_partial_len: usize,
    last_growth: Instant,
    speech_detected: bool,
    last_speech: Instant,
}

/// Обрабатывает одну команду из канала (Stop/TogglePause). Возвращает `true`, если
/// цикл нужно остановить (Stop или Disconnected). Мутирует `state` (пауза/вывод) и
/// управляет cpal-потоком.
fn handle_command(
    cmd_rx: &std_mpsc::Receiver<AudioCommand>,
    stream: &cpal::Stream,
    audio_level: &AtomicU32,
    debug: bool,
    state: &mut LoopState,
) -> Result<bool, ArcanaError> {
    match cmd_rx.try_recv() {
        Ok(AudioCommand::Stop) => return Ok(true),
        Ok(AudioCommand::TogglePause) => {
            if state.paused {
                // Возобновляем
                stream
                    .play()
                    .map_err(|e| ArcanaError::AudioStream(format!("Не удалось возобновить аудиопоток: {}", e)))?;
                state.paused = false;
                state.last_growth = Instant::now();
                if debug {
                    if state.has_output {
                        eprintln!();
                    }
                    eprint!("[Запись] ");
                    state.has_output = false;
                }
            } else {
                // Приостанавливаем
                stream
                    .pause()
                    .map_err(|e| ArcanaError::AudioStream(format!("Не удалось приостановить аудиопоток: {}", e)))?;
                state.paused = true;
                audio_level.store(0, Ordering::Relaxed);
                if debug && state.has_output {
                    eprintln!();
                    state.has_output = false;
                }
                eprintln!("[Пауза]");
            }
        }
        Err(std_mpsc::TryRecvError::Empty) => {}
        Err(std_mpsc::TryRecvError::Disconnected) => return Ok(true),
    }
    Ok(false)
}

/// Останавливает поток, проверяет «мёртвый» микрофон, шлёт Transcribing-event,
/// транскрибирует собранные сэмплы, сохраняет raw-кэш и собирает `RecordResult`.
/// Вынесено из `record_and_transcribe` (пост-loop фаза).
fn finalize_transcription(
    handles: &CaptureHandles,
    transcriber: &dyn Transcriber,
    params: &RecordParams,
    channels: &RecordChannels,
    streaming: bool,
    state: &LoopState,
    recording_start: Instant,
) -> Result<RecordResult, ArcanaError> {
    if params.debug && state.has_output {
        eprintln!();
    }
    eprintln!("[Запись остановлена]");
    channels.audio_level.store(0, Ordering::Relaxed);

    if state.paused {
        let _ = handles.stream.play();
    }
    handles
        .stream
        .pause()
        .map_err(|e| ArcanaError::AudioStream(format!("Не удалось остановить аудиопоток: {}", e)))?;

    let recording_duration = recording_start.elapsed();
    info!(
        "Запись завершена за {:.1}с. Начинаю транскрибацию...",
        recording_duration.as_secs_f64()
    );

    // Проверяем, приходил ли звук с микрофона
    let frames = handles.audio_frames_received.load(Ordering::Relaxed);
    if frames == 0 {
        tracing::warn!(
            "За время записи не получено аудиоданных. Микрофон не подключён или выбрано неверное устройство."
        );
        eprintln!("[Ошибка] Микрофон не захватил звук. Проверьте подключение и настройки аудиоустройства.");
    }

    // Отправляем событие "Транскрибация..." для UI
    let _ = channels.event_tx.send(EngineEvent::Transcribing);

    // Получаем собранные сэмплы
    let samples = handles
        .all_samples
        .lock()
        .map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;

    let transcription_start = std::time::Instant::now();
    // Для streaming (Vosk) — данные уже обработаны через accept_waveform, передаём пустой slice
    // Для batch (Whisper) — передаём все сэмплы
    let transcribe_samples = if streaming { &[][..] } else { &samples[..] };
    let result_text = transcriber.transcribe(transcribe_samples, params.sample_rate)?;
    let transcription_duration = transcription_start.elapsed();
    transcriber.reset();

    if params.debug {
        eprintln!("─────────────────────────────────────────");
        eprintln!(
            "[Результат] {} ({:.1}с)",
            result_text,
            transcription_duration.as_secs_f64()
        );
        eprintln!("─────────────────────────────────────────");
    }
    info!(
        "Финальный результат (запись: {:.1}с, транскрибации: {:.1}с): {}",
        recording_duration.as_secs_f64(),
        transcription_duration.as_secs_f64(),
        result_text
    );

    // Сохраняем аудио в кэш для повторной транскрибации другой моделью
    let timestamp = chrono::Utc::now().timestamp();
    let audio_filename = format!("{}.raw", timestamp);
    let audio_path = params.audio_cache_dir.join(&audio_filename);
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

/// Записывает аудио с микрофона и транскрибирует через выбранный движок.
/// Блокирующая функция — ждёт команд через `cmd_rx`.
/// Автоматически останавливается при тишине (VAD) или по таймауту.
pub fn record_and_transcribe(
    params: RecordParams,
    channels: RecordChannels,
    transcriber: &dyn Transcriber,
) -> Result<RecordResult, ArcanaError> {
    let recording_start = std::time::Instant::now();
    info!("Начинаю запись...");

    let streaming = transcriber.supports_streaming();
    // Настройка и запуск cpal-потока вынесены в build_capture_stream.
    let handles = build_capture_stream(
        params.sample_rate,
        params.mic_gain,
        streaming,
        Arc::clone(&channels.audio_level),
    )?;

    info!("Идет запись... (нажмите хоткей для останова или ждите таймаут)");

    let silence_timeout = Duration::from_secs(params.silence_timeout_secs);
    // VAD: авто-стоп при тишине после речи
    let vad_timeout = Duration::from_secs(params.vad_silence_secs);

    let mut state = LoopState {
        paused: false,
        has_output: false,
        segment_printed: 0,
        max_partial_len: 0,
        last_growth: Instant::now(),
        speech_detected: false,
        last_speech: Instant::now(),
    };

    if params.debug {
        eprint!("[Запись] ");
    }

    loop {
        // Проверяем команды (неблокирующий). true → остановить запись.
        if handle_command(
            &channels.cmd_rx,
            &handles.stream,
            &channels.audio_level,
            params.debug,
            &mut state,
        )? {
            break;
        }

        // Во время паузы не обрабатываем partial и не считаем тишину
        if state.paused {
            thread::sleep(Duration::from_millis(200));
            continue;
        }

        // Для потокового режима (Vosk): передаём данные из канала в транскрайбер
        if let Some(ref rx) = handles.vosk_rx {
            while let Ok(data) = rx.try_recv() {
                if let Err(e) = transcriber.accept_waveform(&data) {
                    tracing::error!("Ошибка при обработке аудиоданных: {}", e);
                }
            }

            // Partial results — только для потокового режима в debug
            if params.debug {
                let partial_text = transcriber.partial_result();
                if !partial_text.is_empty() {
                    let char_count = partial_text.chars().count();
                    if char_count > state.segment_printed {
                        let new_chars: String = partial_text.chars().skip(state.segment_printed).collect();
                        eprint!("{}", new_chars);
                        state.has_output = true;
                        state.segment_printed = char_count;
                    }
                    if partial_text.len() > state.max_partial_len {
                        state.max_partial_len = partial_text.len();
                        state.last_growth = Instant::now();
                    }
                }
            }
        } else {
            // Для пакетного режима (Whisper/GigaAM): таймаут тишины по audio level
            let level = channels.audio_level.load(Ordering::Relaxed);
            if level > 0 {
                state.last_growth = Instant::now();
            }
        }

        // Трекинг речи для VAD: audio level > 5 считается речью
        let current_level = channels.audio_level.load(Ordering::Relaxed);
        if current_level > 5 {
            state.speech_detected = true;
            state.last_speech = Instant::now();
        }

        // Авто-стоп: VAD (речь + тишина) или общий таймаут безопасности.
        match should_stop_on_silence(
            params.vad_enabled,
            state.speech_detected,
            state.last_speech.elapsed(),
            vad_timeout,
            state.last_growth.elapsed(),
            silence_timeout,
        ) {
            Some(StopReason::Vad) => {
                info!("VAD: авто-стоп (речь обнаружена, тишина {}с).", params.vad_silence_secs);
                break;
            }
            Some(StopReason::Timeout) => {
                info!("Запись останавливается по таймауту ({}с).", params.silence_timeout_secs);
                break;
            }
            None => {}
        }

        thread::sleep(Duration::from_millis(200));
    }

    finalize_transcription(
        &handles,
        transcriber,
        &params,
        &channels,
        streaming,
        &state,
        recording_start,
    )
}

/// Сохраняет сырые i16 сэмплы в файл
fn save_raw_audio(path: &std::path::Path, samples: &[i16]) -> Result<(), ArcanaError> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ArcanaError::Internal(format!("Не удалось создать директорию: {}", e)))?;
    }
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut file =
        std::fs::File::create(path).map_err(|e| ArcanaError::Internal(format!("Не удалось создать файл: {}", e)))?;
    file.write_all(&bytes)
        .map_err(|e| ArcanaError::Internal(format!("Не удалось записать аудио: {}", e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_mic_gain_amplifies() {
        let out = apply_mic_gain(&[100, -100], 2.0);
        assert!(matches!(out, Cow::Owned(_)));
        assert_eq!(out.as_ref(), &[200, -200]);
    }

    #[test]
    fn test_apply_mic_gain_saturates() {
        // 30000 * 2 = 60000 → клампится в i16::MAX.
        let out = apply_mic_gain(&[30000], 2.0);
        assert_eq!(out.as_ref(), &[i16::MAX]);
    }

    #[test]
    fn test_apply_mic_gain_passthrough() {
        // gain ≈ 1.0 → без аллокации (borrow).
        let out = apply_mic_gain(&[5, -5], 1.0);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out.as_ref(), &[5, -5]);
    }

    #[test]
    fn test_compute_rms_level() {
        assert_eq!(compute_rms_level(&[0; 160]), (0, false));
        // rms = 3000 → level = (3000/3000).min(1.0)*100 = 100; voiced (rms > 10).
        let (level, voiced) = compute_rms_level(&[3000; 160]);
        assert_eq!(level, 100);
        assert!(voiced);
        // Пустой вход.
        assert_eq!(compute_rms_level(&[]), (0, false));
    }

    #[test]
    fn test_should_stop_on_silence() {
        let s = Duration::from_secs;
        // VAD: речь была, тишина дольше vad_timeout → Vad (проверяется первым).
        assert!(matches!(
            should_stop_on_silence(true, true, s(3), s(2), s(1), s(10)),
            Some(StopReason::Vad)
        ));
        // Общий таймаут: тишины по VAD нет, но last_growth превысил silence_timeout.
        assert!(matches!(
            should_stop_on_silence(false, true, s(5), s(2), s(11), s(10)),
            Some(StopReason::Timeout)
        ));
        // Речи не было — VAD не срабатывает, таймаут не достигнут → None.
        assert!(should_stop_on_silence(true, false, s(9), s(2), s(1), s(10)).is_none());
    }
}
