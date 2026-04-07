// crates/arcanaglyph-core/src/gigaam/transcriber.rs
//
// Транскрайбер на основе GigaAM v3 (ONNX, высокоточный для русского языка).
// Модель: v3_e2e_ctc.int8.onnx (225 МБ), WER ~8.4% на русском.

use std::path::Path;
use std::sync::Mutex;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;

use crate::error::ArcanaError;
use crate::transcriber::{Transcriber, resample, trim_silence};

use super::mel;

/// Транскрайбер на основе GigaAM v3 (SberDevices, ONNX Runtime)
pub struct GigaAmTranscriber {
    session: Mutex<Session>,
    vocab: Vec<String>,
}

impl GigaAmTranscriber {
    /// Создаёт GigaAmTranscriber: загружает ONNX-модель и словарь из директории.
    /// Директория должна содержать v3_e2e_ctc.int8.onnx и v3_e2e_ctc_vocab.txt.
    pub fn new(model_dir: &Path) -> Result<Self, ArcanaError> {
        let onnx_path = model_dir.join("v3_e2e_ctc.int8.onnx");
        let vocab_path = model_dir.join("v3_e2e_ctc_vocab.txt");

        if !onnx_path.exists() {
            return Err(ArcanaError::ModelLoad(format!(
                "ONNX-модель не найдена: {}",
                onnx_path.display()
            )));
        }
        if !vocab_path.exists() {
            return Err(ArcanaError::ModelLoad(format!(
                "Словарь не найден: {}",
                vocab_path.display()
            )));
        }

        tracing::info!("Загрузка GigaAM v3 из: {:?}", model_dir);

        let n_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        let session = Session::builder()
            .map_err(|e| ArcanaError::ModelLoad(format!("Ошибка создания ONNX Session builder: {}", e)))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| ArcanaError::ModelLoad(format!("Ошибка установки уровня оптимизации: {}", e)))?
            .with_intra_threads(n_threads)
            .map_err(|e| ArcanaError::ModelLoad(format!("Ошибка установки потоков: {}", e)))?
            .commit_from_file(&onnx_path)
            .map_err(|e| ArcanaError::ModelLoad(format!("Не удалось загрузить ONNX-модель: {}", e)))?;

        let vocab_content = std::fs::read_to_string(&vocab_path)
            .map_err(|e| ArcanaError::ModelLoad(format!("Не удалось прочитать словарь: {}", e)))?;
        // Формат строки: "токен индекс" (например "▁при 134" или "<blk> 256")
        // Берём только первое слово — сам токен
        let vocab: Vec<String> = vocab_content
            .lines()
            .filter(|s| !s.is_empty())
            .map(|s| s.split_whitespace().next().unwrap_or("").to_string())
            .collect();

        tracing::info!(
            "GigaAM v3 загружена: {} токенов в словаре, {} потоков",
            vocab.len(),
            n_threads
        );

        Ok(Self {
            session: Mutex::new(session),
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

        // Вычисляем mel-спектрограмму [1, 64, T_frames]
        let mel_spec = mel::compute_mel_spectrogram(&audio_f32);
        let n_frames = mel_spec.shape()[2];

        if n_frames == 0 {
            return Ok(String::new());
        }

        // Подготовка входных тензоров для ONNX Runtime
        let mel_contiguous = mel_spec.as_standard_layout();
        let mel_tensor = TensorRef::from_array_view(mel_contiguous.view())
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка создания mel тензора: {}", e)))?;

        let length = ndarray::Array1::from_vec(vec![n_frames as i64]);
        let length_tensor = TensorRef::from_array_view(length.view())
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка создания length тензора: {}", e)))?;

        let mut session = self.session.lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;

        let outputs = session
            .run(ort::inputs![
                "features" => mel_tensor,
                "feature_lengths" => length_tensor
            ])
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка ONNX inference: {}", e)))?;

        // Извлекаем логиты [1, T/4, vocab_size]
        let (shape, logits_data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка извлечения выходного тензора: {}", e)))?;

        // shape = [1, T_out, vocab_size] → обрабатываем как 2D [T_out, vocab_size]
        let t_out = shape[1] as usize;
        let vocab_size = shape[2] as usize;
        let text = ctc_greedy_decode(logits_data, t_out, vocab_size, &self.vocab);

        Ok(text)
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

/// CTC greedy decode: argmax по кадрам, удаление дубликатов и blank-токенов.
/// logits_data — плоский массив [T_out * vocab_size], row-major.
fn ctc_greedy_decode(logits_data: &[f32], t_out: usize, vocab_size: usize, vocab: &[String]) -> String {
    // blank = токен "<blk>" (обычно последний в словаре, индекс 256)
    let blank_id = vocab.iter().position(|t| t == "<blk>").unwrap_or(vocab.len() - 1);
    let mut token_ids = Vec::new();
    let mut prev_tok = blank_id;

    for t in 0..t_out {
        let frame = &logits_data[t * vocab_size..(t + 1) * vocab_size];

        // argmax
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

    // Собираем текст из subword-токенов
    let mut text = String::new();
    for &id in &token_ids {
        if id < vocab.len() {
            let token = &vocab[id];
            // Пропускаем служебные токены
            if token == "<blk>" || token == "<unk>" {
                continue;
            }
            text.push_str(token);
        }
    }

    // SentencePiece: символ ▁ → пробел
    text = text.replace('\u{2581}', " ");
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ctc_greedy_decode_basic() {
        let vocab: Vec<String> = vec![
            "\u{2581}при".to_string(),  // 0
            "вет".to_string(),           // 1
            "\u{2581}мир".to_string(),   // 2
            "<blk>".to_string(),         // 3 (blank)
        ];
        // blank_id = 3 (токен "<blk>"), vocab_size = 4

        // Кадры с argmax = [0, 0, 3, 1, 3, 2]
        // После dedupe: [0, 1, 2] → "▁привет▁мир" → " привет мир" → "привет мир"
        #[rustfmt::skip]
        let logits: Vec<f32> = vec![
            10.0, 0.0, 0.0, 0.0,  // → 0
            10.0, 0.0, 0.0, 0.0,  // → 0 (дубликат, пропуск)
            0.0, 0.0, 0.0, 10.0,  // → 3 (blank)
            0.0, 10.0, 0.0, 0.0,  // → 1
            0.0, 0.0, 0.0, 10.0,  // → 3 (blank)
            0.0, 0.0, 10.0, 0.0,  // → 2
        ];

        let result = ctc_greedy_decode(&logits, 6, 4, &vocab);
        assert_eq!(result, "привет мир");
    }

    #[test]
    fn test_ctc_greedy_decode_empty() {
        let vocab: Vec<String> = vec!["а".to_string(), "<blk>".to_string()];
        // Все кадры = blank (индекс 1)
        let logits: Vec<f32> = vec![
            0.0, 10.0,
            0.0, 10.0,
        ];
        let result = ctc_greedy_decode(&logits, 2, 2, &vocab);
        assert_eq!(result, "");
    }
}
