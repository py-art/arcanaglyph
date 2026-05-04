// crates/arcanaglyph-core/src/gigaam/transcriber_tract.rs
//
// Альтернативный транскрайбер GigaAM v3 на pure-Rust ONNX-runtime tract.
// Используется на CPU без AVX (например Intel Celeron N5095): pre-built Microsoft
// ONNX Runtime, который качает `ort` крейт, требует AVX SIMD и крашит SIGILL до
// main() на CPU без AVX. tract — чистый Rust, использует только базовые SIMD/SSE
// инструкции и работает на любом x86_64 / aarch64 CPU.
//
// Модель: v3_e2e_ctc.onnx (FP32, ~846 МБ). INT8-квантизованная версия (используемая
// в `transcriber.rs`/ort) даёт меньший размер, но tract имеет ограниченную
// поддержку Q8-операций. FP32 универсально совместима.
//
// Алгоритм идентичен `transcriber.rs`: трим тишины → ресемпл 16kHz → mel-спектр →
// CTC inference → greedy decode → SentencePiece detokenize.

use std::path::Path;
use std::sync::Mutex;

use tract_onnx::prelude::*;

use crate::error::ArcanaError;
use crate::transcriber::{Transcriber, resample, trim_silence};

use super::mel;

/// Тип runnable-модели tract — typed (с известными формами тензоров) для
/// максимальной производительности после `into_optimized()`.
type RunnableModel = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

/// Транскрайбер на основе GigaAM v3 через tract (pure-Rust ONNX inference)
pub struct GigaAmTranscriber {
    model: Mutex<RunnableModel>,
    vocab: Vec<String>,
}

impl GigaAmTranscriber {
    /// Создаёт GigaAmTranscriber: загружает FP32 ONNX-модель и словарь.
    /// Директория должна содержать v3_e2e_ctc.onnx и v3_e2e_ctc_vocab.txt.
    pub fn new(model_dir: &Path) -> Result<Self, ArcanaError> {
        let onnx_path = model_dir.join("v3_e2e_ctc.onnx");
        let vocab_path = model_dir.join("v3_e2e_ctc_vocab.txt");

        if !onnx_path.exists() {
            return Err(ArcanaError::ModelLoad(format!(
                "ONNX-модель не найдена: {} (нужна FP32 версия для tract-backend)",
                onnx_path.display()
            )));
        }
        if !vocab_path.exists() {
            return Err(ArcanaError::ModelLoad(format!(
                "Словарь не найден: {}",
                vocab_path.display()
            )));
        }

        tracing::info!("Загрузка GigaAM v3 (tract-backend, FP32) из: {:?}", model_dir);

        // Загружаем модель без задания фиксированных input facts —
        // используем то, что уже описано в самом ONNX. tract извлечёт
        // shape из модели и будет работать с динамическим временным измерением.
        let model = tract_onnx::onnx()
            .model_for_path(&onnx_path)
            .map_err(|e| ArcanaError::ModelLoad(format!("Не удалось загрузить ONNX-модель через tract: {}", e)))?
            .into_optimized()
            .map_err(|e| ArcanaError::ModelLoad(format!("Ошибка оптимизации ONNX-модели tract'ом: {}", e)))?
            .into_runnable()
            .map_err(|e| ArcanaError::ModelLoad(format!("Не удалось получить runnable модель из tract: {}", e)))?;

        let vocab_content = std::fs::read_to_string(&vocab_path)
            .map_err(|e| ArcanaError::ModelLoad(format!("Не удалось прочитать словарь: {}", e)))?;
        // Формат строки: "токен индекс" (например "▁при 134" или "<blk> 256")
        let vocab: Vec<String> = vocab_content
            .lines()
            .filter(|s| !s.is_empty())
            .map(|s| s.split_whitespace().next().unwrap_or("").to_string())
            .collect();

        tracing::info!("GigaAM v3 (tract-backend) загружена: {} токенов в словаре", vocab.len(),);

        Ok(Self {
            model: Mutex::new(model),
            vocab,
        })
    }
}

impl Transcriber for GigaAmTranscriber {
    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> Result<String, ArcanaError> {
        // Обрезаем тишину
        let trimmed = trim_silence(samples, sample_rate);

        // i16 → f32 (нормализация в [-1.0, 1.0])
        let mut audio_f32: Vec<f32> = trimmed.iter().map(|&s| s as f32 / 32768.0).collect();

        // Ресемплируем до 16 kHz если нужно
        if sample_rate != 16000 {
            audio_f32 = resample(&audio_f32, sample_rate, 16000);
        }

        if audio_f32.len() < 320 {
            return Ok(String::new());
        }

        // Mel-спектрограмма [1, 64, T_frames] (тот же модуль, что и для ort-backend)
        let mel_spec = mel::compute_mel_spectrogram(&audio_f32);
        let n_frames = mel_spec.shape()[2];

        if n_frames == 0 {
            return Ok(String::new());
        }

        // Конвертируем ndarray (наша версия 0.17) в tract-тензор через flat data:
        // tract использует внутри собственный tract_ndarray от другой версии,
        // прямой `into_tensor()` не работает между разными версиями.
        // Для standard layout (row-major) iter дают данные в правильном C-order порядке.
        let mel_contiguous = mel_spec.as_standard_layout();
        let mel_shape: Vec<usize> = mel_contiguous.shape().to_vec();
        let mel_data: Vec<f32> = mel_contiguous.iter().copied().collect();
        let mel_tensor = Tensor::from_shape(&mel_shape, &mel_data)
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка создания mel-тензора tract: {}", e)))?;

        // feature_lengths: [1] i64 — длина последовательности в кадрах.
        let lengths_data: Vec<i64> = vec![n_frames as i64];
        let lengths = Tensor::from_shape(&[1usize], &lengths_data)
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка создания length-тензора tract: {}", e)))?;

        let model = self
            .model
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;

        let outputs = model
            .run(tvec!(mel_tensor.into(), lengths.into()))
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка tract inference: {}", e)))?;

        // Извлекаем логиты: ожидаем форму [1, T_out, vocab_size] f32.
        let logits_view = outputs[0]
            .to_array_view::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка извлечения логитов: {}", e)))?;

        let shape = logits_view.shape();
        if shape.len() != 3 {
            return Err(ArcanaError::Recognizer(format!(
                "Неожиданная форма выходного тензора tract: {:?}",
                shape
            )));
        }
        let t_out = shape[1];
        let vocab_size = shape[2];

        // Получаем плоский срез данных. Если view не contiguous — .to_owned() копирует
        // в стандартный layout (row-major).
        let logits_owned = logits_view.to_owned();
        let logits_slice = logits_owned
            .as_slice()
            .ok_or_else(|| ArcanaError::Recognizer("Логиты tract не contiguous после to_owned()".into()))?;

        let text = ctc_greedy_decode(logits_slice, t_out, vocab_size, &self.vocab);
        Ok(text)
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

/// CTC greedy decode: argmax по кадрам, удаление дубликатов и blank-токенов.
/// Идентично `transcriber.rs` (ort-backend) — алгоритм одинаковый.
fn ctc_greedy_decode(logits_data: &[f32], t_out: usize, vocab_size: usize, vocab: &[String]) -> String {
    let blank_id = vocab.iter().position(|t| t == "<blk>").unwrap_or(vocab.len() - 1);
    let mut token_ids = Vec::new();
    let mut prev_tok = blank_id;

    for t in 0..t_out {
        let frame = &logits_data[t * vocab_size..(t + 1) * vocab_size];
        let tok = frame
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx)
            .unwrap_or(blank_id);

        if tok != blank_id && (tok != prev_tok || prev_tok == blank_id) {
            token_ids.push(tok);
        }
        prev_tok = tok;
    }

    let mut text = String::new();
    for &id in &token_ids {
        if id < vocab.len() {
            let token = &vocab[id];
            if token == "<blk>" || token == "<unk>" {
                continue;
            }
            text.push_str(token);
        }
    }

    text = text.replace('\u{2581}', " ");
    text.trim().to_string()
}
