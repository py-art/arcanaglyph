// crates/arcanaglyph-core/src/gigaam/transcriber_rnnt.rs
//
// Транскрайбер на основе GigaAM v3 E2E RNN-T (ONNX, высокоточный для русского).
// Точнее CTC-варианта (WER ~8.4% vs ~9.2%), но тяжелее: вместо одного argmax-
// прохода — авторегрессивный greedy-декод (encoder → frame-loop с decoder + joint).
//
// Модель — три ONNX-файла (istupakov/gigaam-v3-onnx, INT8):
//   v3_e2e_rnnt_encoder.int8.onnx — audio_signal[1,64,T],length[1] → encoded[1,768,T'],encoded_len
//   v3_e2e_rnnt_decoder.int8.onnx — x:i64[1,1], h.1/c.1:f32[1,1,320] → dec:f32[1,1,320], h, c
//   v3_e2e_rnnt_joint.int8.onnx   — enc:f32[1,768,1], dec:f32[1,320,1] → joint:f32[1,1,1,1025]
// + словарь v3_e2e_rnnt_vocab.txt (1025 строк, индексы 0..1024; blank "<blk>" = последний, 1024).

use std::path::Path;
use std::sync::Mutex;

#[cfg(feature = "cuda")]
use ort::execution_providers::CUDAExecutionProvider;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;

use crate::dsp::preprocess_to_f32_16k;
use crate::error::ArcanaError;
use crate::transcriber::Transcriber;

use super::mel;

// Размерности из ONNX-графов (фиксированы экспортом istupakov).
const ENC_DIM: usize = 768; // размер кадра энкодера
const PRED_HIDDEN: usize = 320; // скрытое состояние prediction-network (LSTM)
const MAX_TOKENS_PER_STEP: usize = 3; // потолок эмиссий на один кадр энкодера

/// Транскрайбер GigaAM v3 E2E RNN-T (SberDevices, ONNX Runtime).
pub struct GigaAmRnntTranscriber {
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
    joint: Mutex<Session>,
    vocab: Vec<String>,
}

impl GigaAmRnntTranscriber {
    /// Создаёт транскрайбер: загружает encoder/decoder/joint ONNX и словарь.
    /// Директория должна содержать четыре файла v3_e2e_rnnt_*.
    pub fn new(model_dir: &Path) -> Result<Self, ArcanaError> {
        let encoder_path = model_dir.join("v3_e2e_rnnt_encoder.int8.onnx");
        let decoder_path = model_dir.join("v3_e2e_rnnt_decoder.int8.onnx");
        let joint_path = model_dir.join("v3_e2e_rnnt_joint.int8.onnx");
        let vocab_path = model_dir.join("v3_e2e_rnnt_vocab.txt");

        for p in [&encoder_path, &decoder_path, &joint_path, &vocab_path] {
            if !p.exists() {
                return Err(ArcanaError::ModelLoad(format!(
                    "Файл модели не найден: {}",
                    p.display()
                )));
            }
        }

        tracing::info!("Загрузка GigaAM v3 RNN-T из: {:?}", model_dir);

        let n_threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let encoder = build_session(&encoder_path, n_threads)?;
        let decoder = build_session(&decoder_path, n_threads)?;
        let joint = build_session(&joint_path, n_threads)?;

        let vocab_content = std::fs::read_to_string(&vocab_path)
            .map_err(|e| ArcanaError::ModelLoad(format!("Не удалось прочитать словарь: {}", e)))?;
        // Формат строки: "токен индекс" — берём первое слово (сам токен).
        let vocab: Vec<String> = vocab_content
            .lines()
            .filter(|s| !s.is_empty())
            .map(|s| s.split_whitespace().next().unwrap_or("").to_string())
            .collect();

        tracing::info!(
            "GigaAM v3 RNN-T загружена: {} токенов, {} потоков",
            vocab.len(),
            n_threads
        );

        Ok(Self {
            encoder: Mutex::new(encoder),
            decoder: Mutex::new(decoder),
            joint: Mutex::new(joint),
            vocab,
        })
    }
}

/// Собирает ONNX Session с тем же выбором opt-level по backend, что и CTC-вариант.
fn build_session(path: &Path, n_threads: usize) -> Result<Session, ArcanaError> {
    #[allow(unused_mut)]
    let mut builder = Session::builder()
        .map_err(|e| ArcanaError::ModelLoad(format!("Ошибка создания ONNX Session builder: {}", e)))?;

    #[cfg(feature = "cuda")]
    {
        builder = builder
            .with_execution_providers([CUDAExecutionProvider::default().build()])
            .map_err(|e| ArcanaError::ModelLoad(format!("Ошибка настройки CUDA EP: {}", e)))?;
    }

    // gigaam-system-ort (без AVX) — Disable; gigaam (Microsoft ORT, AVX) — Level3.
    #[cfg(feature = "gigaam-system-ort")]
    let opt_level = GraphOptimizationLevel::Disable;
    #[cfg(all(feature = "gigaam", not(feature = "gigaam-system-ort")))]
    let opt_level = GraphOptimizationLevel::Level3;

    builder
        .with_optimization_level(opt_level)
        .map_err(|e| ArcanaError::ModelLoad(format!("Ошибка установки уровня оптимизации: {}", e)))?
        .with_intra_threads(n_threads)
        .map_err(|e| ArcanaError::ModelLoad(format!("Ошибка установки потоков: {}", e)))?
        .commit_from_file(path)
        .map_err(|e| ArcanaError::ModelLoad(format!("Не удалось загрузить ONNX-модель {}: {}", path.display(), e)))
}

impl Transcriber for GigaAmRnntTranscriber {
    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> Result<String, ArcanaError> {
        // Общий препроцессинг: обрезка тишины + i16→f32 + resample до 16 кГц.
        let audio_f32 = preprocess_to_f32_16k(samples, sample_rate);
        if audio_f32.len() < 320 {
            return Ok(String::new());
        }

        // mel-спектрограмма [1, 64, T_frames] — тот же препроцессинг, что у CTC.
        let mel_spec = mel::compute_mel_spectrogram(&audio_f32);
        let n_frames = mel_spec.shape()[2];
        if n_frames == 0 {
            return Ok(String::new());
        }

        // --- 1. Энкодер: audio_signal + length → encoded [1, 768, T'] ---
        let encoded = {
            let mel_contiguous = mel_spec.as_standard_layout();
            let audio_tensor = TensorRef::from_array_view(mel_contiguous.view())
                .map_err(|e| ArcanaError::Recognizer(format!("Ошибка mel-тензора: {}", e)))?;
            let length = ndarray::Array1::from_vec(vec![n_frames as i64]);
            let length_tensor = TensorRef::from_array_view(length.view())
                .map_err(|e| ArcanaError::Recognizer(format!("Ошибка length-тензора: {}", e)))?;

            let mut enc = self
                .encoder
                .lock()
                .map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;
            let outputs = enc
                .run(ort::inputs!["audio_signal" => audio_tensor, "length" => length_tensor])
                .map_err(|e| ArcanaError::Recognizer(format!("Ошибка encoder inference: {}", e)))?;

            let (shape, data) = outputs["encoded"]
                .try_extract_tensor::<f32>()
                .map_err(|e| ArcanaError::Recognizer(format!("Ошибка извлечения encoded: {}", e)))?;
            // Число валидных кадров берём из encoded_len (а не из формы — паддинг).
            let t_valid = outputs["encoded_len"]
                .try_extract_tensor::<i64>()
                .ok()
                .and_then(|(_, l)| l.first().copied())
                .map(|v| v as usize);
            extract_encoder_frames(shape, data, t_valid)
        };
        if encoded.is_empty() {
            return Ok(String::new());
        }

        // --- 2. Greedy RNN-T декод по кадрам энкодера ---
        let token_ids = self.greedy_decode(&encoded)?;
        let text = rnnt_tokens_to_text(&token_ids, &self.vocab);
        Ok(text)
    }
}

impl GigaAmRnntTranscriber {
    /// Greedy transducer decode: по каждому кадру энкодера крутим decoder+joint,
    /// эмитим до MAX_TOKENS_PER_STEP не-blank токенов, обновляя LSTM-состояние.
    fn greedy_decode(&self, encoded: &[Vec<f32>]) -> Result<Vec<usize>, ArcanaError> {
        let mut decoder = self
            .decoder
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;
        let mut joint = self
            .joint
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;

        // blank = индекс токена "<blk>" в словаре (у v3 RNN-T это последняя строка,
        // индекс 1024; словарь включает blank, поэтому len = joint_out = 1025).
        // Он же служит SOS-токеном для стартового прогона decoder. Fallback на
        // последний индекс — на случай иного формата словаря.
        let blank_id = self
            .vocab
            .iter()
            .position(|t| t == "<blk>")
            .unwrap_or(self.vocab.len().saturating_sub(1));

        // LSTM-состояние prediction-network инициализируем нулями.
        let mut h = vec![0.0f32; PRED_HIDDEN];
        let mut c = vec![0.0f32; PRED_HIDDEN];
        // Стартовый прогон decoder с blank/SOS токеном.
        let (mut dec, nh, nc) = run_decoder(&mut decoder, blank_id as i64, &h, &c)?;
        h = nh;
        c = nc;

        let mut hyp: Vec<usize> = Vec::new();

        for enc_t in encoded {
            let mut emitted = 0usize;
            while emitted < MAX_TOKENS_PER_STEP {
                let logits = run_joint(&mut joint, enc_t, &dec)?;
                let k = argmax(&logits);
                if k == blank_id {
                    break;
                }
                hyp.push(k);
                let (ndec, nh, nc) = run_decoder(&mut decoder, k as i64, &h, &c)?;
                dec = ndec;
                h = nh;
                c = nc;
                emitted += 1;
            }
        }
        Ok(hyp)
    }
}

/// Извлекает кадры энкодера в Vec векторов длины ENC_DIM. Форма ONNX —
/// [1, 768, T] (D-major, как транспонирует onnx-asr): кадр t = data[d*T + t].
/// На случай иной раскладки детектируем, какая ось равна 768.
fn extract_encoder_frames(shape: &[i64], data: &[f32], t_valid: Option<usize>) -> Vec<Vec<f32>> {
    if shape.len() != 3 {
        return Vec::new();
    }
    let (d1, d2) = (shape[1] as usize, shape[2] as usize);
    // Раскладка [1, D, T] (ожидаемая): ось 1 = ENC_DIM, ось 2 = время.
    let (t_total, d_major) = if d1 == ENC_DIM {
        (d2, true)
    } else if d2 == ENC_DIM {
        (d1, false)
    } else {
        return Vec::new();
    };
    let t = t_valid.map(|v| v.min(t_total)).unwrap_or(t_total);
    let mut frames = Vec::with_capacity(t);
    for ti in 0..t {
        let mut frame = vec![0.0f32; ENC_DIM];
        for (d, slot) in frame.iter_mut().enumerate() {
            // d_major: [1,D,T] → idx = d*T + ti; иначе [1,T,D] → idx = ti*D + d.
            *slot = if d_major {
                data[d * t_total + ti]
            } else {
                data[ti * ENC_DIM + d]
            };
        }
        frames.push(frame);
    }
    frames
}

/// Выход decoder: (dec, новое h, новое c) — все длины PRED_HIDDEN.
type DecoderStep = (Vec<f32>, Vec<f32>, Vec<f32>);

/// Прогон decoder: токен x:i64[1,1] + состояния h.1/c.1:f32[1,1,320] → dec, h, c.
fn run_decoder(decoder: &mut Session, token: i64, h: &[f32], c: &[f32]) -> Result<DecoderStep, ArcanaError> {
    let x = ndarray::Array2::<i64>::from_elem((1, 1), token);
    let h_arr = ndarray::Array3::<f32>::from_shape_vec((1, 1, PRED_HIDDEN), h.to_vec())
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка h-тензора: {}", e)))?;
    let c_arr = ndarray::Array3::<f32>::from_shape_vec((1, 1, PRED_HIDDEN), c.to_vec())
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка c-тензора: {}", e)))?;

    let x_ref = TensorRef::from_array_view(x.view())
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка x-тензора: {}", e)))?;
    let h_ref = TensorRef::from_array_view(h_arr.view())
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка h-view: {}", e)))?;
    let c_ref = TensorRef::from_array_view(c_arr.view())
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка c-view: {}", e)))?;

    let outputs = decoder
        .run(ort::inputs!["x" => x_ref, "h.1" => h_ref, "c.1" => c_ref])
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка decoder inference: {}", e)))?;

    // Замыкание-экстрактор: убирает тройное повторение try_extract+map_err+to_vec.
    let extract = |name: &str| -> Result<Vec<f32>, ArcanaError> {
        Ok(outputs[name]
            .try_extract_tensor::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка извлечения {}: {}", name, e)))?
            .1
            .to_vec())
    };
    Ok((extract("dec")?, extract("h")?, extract("c")?))
}

/// Прогон joint: enc:f32[1,768,1] + dec:f32[1,320,1] → плоский вектор логитов (1025).
fn run_joint(joint: &mut Session, enc_frame: &[f32], dec: &[f32]) -> Result<Vec<f32>, ArcanaError> {
    let enc_arr = ndarray::Array3::<f32>::from_shape_vec((1, ENC_DIM, 1), enc_frame.to_vec())
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка enc-тензора: {}", e)))?;
    // dec из decoder идёт [1,1,320] → joint ждёт [1,320,1]; данные те же 320 значений.
    let dec_arr = ndarray::Array3::<f32>::from_shape_vec((1, PRED_HIDDEN, 1), dec.to_vec())
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка dec-тензора: {}", e)))?;

    let enc_ref = TensorRef::from_array_view(enc_arr.view())
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка enc-view: {}", e)))?;
    let dec_ref = TensorRef::from_array_view(dec_arr.view())
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка dec-view: {}", e)))?;

    let outputs = joint
        .run(ort::inputs!["enc" => enc_ref, "dec" => dec_ref])
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка joint inference: {}", e)))?;
    let logits = outputs["joint"]
        .try_extract_tensor::<f32>()
        .map_err(|e| ArcanaError::Recognizer(format!("Ошибка извлечения joint: {}", e)))?
        .1
        .to_vec();
    Ok(logits)
}

/// argmax по плоскому вектору логитов.
fn argmax(logits: &[f32]) -> usize {
    logits
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Собирает текст из subword-токенов (SentencePiece: ▁ → пробел).
fn rnnt_tokens_to_text(token_ids: &[usize], vocab: &[String]) -> String {
    let mut text = String::new();
    for &id in token_ids {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argmax_picks_max_index() {
        assert_eq!(argmax(&[0.1, 0.9, 0.3]), 1);
        assert_eq!(argmax(&[5.0, 1.0, 1.0]), 0);
        assert_eq!(argmax(&[1.0, 1.0, 9.0]), 2);
    }

    #[test]
    fn tokens_to_text_joins_sentencepiece() {
        let vocab: Vec<String> = vec![
            "\u{2581}при".to_string(), // 0
            "вет".to_string(),         // 1
            "\u{2581}мир".to_string(), // 2
            "<unk>".to_string(),       // 3
        ];
        // [0,1,2] → "▁привет▁мир" → " привет мир" → "привет мир"
        assert_eq!(rnnt_tokens_to_text(&[0, 1, 2], &vocab), "привет мир");
        // <unk> пропускается, out-of-range id игнорируется
        assert_eq!(rnnt_tokens_to_text(&[0, 3, 99], &vocab), "при");
    }

    #[test]
    fn extract_frames_d_major_layout() {
        // shape [1, 768, T] — но проверим на маленьком ENC_DIM-несовпадении:
        // здесь d1 != ENC_DIM и d2 != ENC_DIM → пустой результат (страховка).
        let data = vec![0.0f32; 6];
        assert!(extract_encoder_frames(&[1, 2, 3], &data, None).is_empty());
        // некорректная размерность формы → пусто
        assert!(extract_encoder_frames(&[1, 768], &data, Some(1)).is_empty());
    }

    // Интеграционный smoke-test: требует скачанную модель в каталоге из
    // ARCANA_RNNT_MODEL_DIR (4 файла v3_e2e_rnnt_*). Гоняет весь путь
    // encoder→decoder→joint→greedy на синтетическом сигнале — ловит ошибки
    // имён/форм ONNX-тензоров (главный риск реализации) без микрофона.
    // Запуск:
    //   ARCANA_RNNT_MODEL_DIR=~/.local/share/arcanaglyph/models/gigaam-v3-e2e-rnnt \
    //   LIBRARY_PATH=/usr/local/lib cargo test -p arcanaglyph-core \
    //     transcriber_rnnt::tests::smoke -- --ignored --nocapture
    #[test]
    #[ignore = "требует скачанную RNN-T модель в ARCANA_RNNT_MODEL_DIR"]
    fn smoke_transcribe_runs_without_shape_errors() {
        let dir = match std::env::var("ARCANA_RNNT_MODEL_DIR") {
            Ok(d) => d,
            Err(_) => return,
        };
        let transcriber = GigaAmRnntTranscriber::new(std::path::Path::new(&dir)).expect("загрузка RNN-T модели");
        // 1.5 с синтетического сигнала (сумма синусов) @16кГц i16.
        let n = 24_000usize;
        let samples: Vec<i16> = (0..n)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                let s = 0.3 * (2.0 * std::f32::consts::PI * 180.0 * t).sin()
                    + 0.2 * (2.0 * std::f32::consts::PI * 320.0 * t).sin();
                (s * 12_000.0) as i16
            })
            .collect();
        let out = transcriber.transcribe(&samples, 16_000).expect("transcribe вернул Err");
        // Текст может быть пустым/мусорным на синтетике — важно, что путь прошёл
        // без ORT shape-ошибок и greedy-loop отработал.
        println!("RNN-T smoke output: {out:?}");
    }
}
