// crates/arcanaglyph-core/src/qwen3asr/transcriber.rs
//
// Транскрайбер Qwen3-ASR-0.6B (ONNX, мультиязычный, авторегрессивный decoder).
// 4 ONNX-файла: encoder_conv, encoder_transformer, decoder_init, decoder_step
// + embed_tokens.bin (матрица эмбеддингов) + tokenizer.json (BPE)

use std::path::Path;
use std::sync::Mutex;

use ndarray::{Array2, Array4};
#[cfg(feature = "cuda")]
use ort::execution_providers::CUDAExecutionProvider;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::TensorRef;

use crate::dsp::preprocess_to_f32_16k;
use crate::error::ArcanaError;
use crate::transcriber::Transcriber;

use super::mel;

// Специальные токены Qwen3-ASR
const AUDIO_START_ID: i64 = 151669;
const AUDIO_END_ID: i64 = 151670;
const AUDIO_PAD_ID: i64 = 151676;
const IM_START_ID: i64 = 151644;
const IM_END_ID: i64 = 151645;
const ENDOFTEXT_ID: i64 = 151643;
const NEWLINE_ID: i64 = 198;

const VOCAB_SIZE: usize = 151936;
const HIDDEN_SIZE: usize = 1024;
const CHUNK_SIZE: usize = 100;
const N_MELS: usize = 128;

/// Greedy argmax по slice логитов → token id. Пустой slice или NaN-only →
/// fallback `ENDOFTEXT_ID`. Дедуплицирует argmax из prefill и шага декодера.
/// Чистая функция (тестируется без ONNX).
fn argmax_token(logits: &[f32]) -> i64 {
    logits
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(idx, _)| idx as i64)
        .unwrap_or(ENDOFTEXT_ID)
}

/// Вырезает служебный префикс (language tag) до и включая `<asr_text>` и тримит.
/// Если тега нет — просто тримит. Чистая функция.
fn strip_asr_prefix(decoded: &str) -> String {
    if let Some(idx) = decoded.find("<asr_text>") {
        decoded[idx + 10..].trim().to_string()
    } else {
        decoded.trim().to_string()
    }
}

/// Проверяет наличие всех обязательных ONNX-файлов в `onnx_dir` и `tokenizer.json`
/// в `model_dir`. Возвращает путь к токенизатору. Вынесено из `new`.
fn check_required_files(onnx_dir: &Path, model_dir: &Path) -> Result<std::path::PathBuf, ArcanaError> {
    let required = [
        "encoder_conv.onnx",
        "encoder_transformer.onnx",
        "decoder_init.int8.onnx",
        "decoder_step.int8.onnx",
        "embed_tokens.bin",
    ];
    for f in &required {
        let p = onnx_dir.join(f);
        if !p.exists() {
            return Err(ArcanaError::ModelLoad(format!("Файл не найден: {}", p.display())));
        }
    }
    let tokenizer_path = model_dir.join("tokenizer.json");
    if !tokenizer_path.exists() {
        return Err(ArcanaError::ModelLoad(format!(
            "tokenizer.json не найден: {}",
            tokenizer_path.display()
        )));
    }
    Ok(tokenizer_path)
}

/// Загружает матрицу эмбеддингов [VOCAB_SIZE, HIDDEN_SIZE] f32 из `embed_tokens.bin`,
/// проверяя ожидаемый размер байт. Вынесено из `new`.
fn load_embeddings(onnx_dir: &Path) -> Result<Vec<f32>, ArcanaError> {
    let embed_path = onnx_dir.join("embed_tokens.bin");
    let embed_bytes =
        std::fs::read(&embed_path).map_err(|e| ArcanaError::ModelLoad(format!("embed_tokens.bin: {}", e)))?;
    let expected_size = VOCAB_SIZE * HIDDEN_SIZE * 4;
    if embed_bytes.len() != expected_size {
        return Err(ArcanaError::ModelLoad(format!(
            "embed_tokens.bin: ожидается {} байт, получено {}",
            expected_size,
            embed_bytes.len()
        )));
    }
    // Безопасно конвертируем байты в f32
    Ok(embed_bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// Упаковывает mel [128, T] в padded чанки [chunk_num, 1, 128, max_chunk_len].
/// Чистая функция (без ONNX) — тестируема.
fn pack_padded_chunks(mel: &Array2<f32>, chunk_lengths: &[usize], max_chunk_len: usize) -> Array4<f32> {
    let chunk_num = chunk_lengths.len();
    let mut padded = Array4::<f32>::zeros((chunk_num, 1, N_MELS, max_chunk_len));
    let mut start = 0;
    for (i, &cl) in chunk_lengths.iter().enumerate() {
        for m in 0..N_MELS {
            for t in 0..cl {
                padded[[i, 0, m, t]] = mel[[m, start + t]];
            }
        }
        start += cl;
    }
    padded
}

/// Распаковывает выход conv-frontend ([chunk_num * conv_tokens_per_chunk, conv_dim]
/// в виде плоского `conv_data`) в плотную матрицу [total_tokens, conv_dim], убирая
/// padding по `lens_after_cnn`. Чистая функция — тестируема.
fn unpack_conv_output(
    conv_data: &[f32],
    conv_tokens_per_chunk: usize,
    conv_dim: usize,
    lens_after_cnn: &[usize],
) -> Array2<f32> {
    let total_tokens: usize = lens_after_cnn.iter().sum();
    let mut hidden_states = Array2::<f32>::zeros((total_tokens, conv_dim));
    let mut dst = 0;
    for (i, &l) in lens_after_cnn.iter().enumerate() {
        let src_offset = i * conv_tokens_per_chunk * conv_dim;
        for t in 0..l {
            for d in 0..conv_dim {
                hidden_states[[dst + t, d]] = conv_data[src_offset + t * conv_dim + d];
            }
        }
        dst += l;
    }
    hidden_states
}

/// KV-cache декодера, прокидываемый между шагами авторегрессии. `keys_shape`
/// используется и для keys, и для values (как в исходном коде).
struct KvCache {
    keys: Vec<f32>,
    values: Vec<f32>,
    keys_shape: Vec<usize>,
}

/// Транскрайбер Qwen3-ASR-0.6B (мультиязычный, ONNX Runtime)
pub struct Qwen3AsrTranscriber {
    encoder_conv: Mutex<Session>,
    encoder_transformer: Mutex<Session>,
    decoder_init: Mutex<Session>,
    decoder_step: Mutex<Session>,
    embed_tokens: Vec<f32>, // [VOCAB_SIZE * HIDDEN_SIZE]
    tokenizer: tokenizers::Tokenizer,
    // Закэшированные ID токенов "system", "user", "assistant"
    system_ids: Vec<i64>,
    user_ids: Vec<i64>,
    assistant_ids: Vec<i64>,
}

impl Qwen3AsrTranscriber {
    /// Создаёт Qwen3AsrTranscriber: загружает 4 ONNX-модели, эмбеддинги и токенизатор.
    /// Директория должна содержать:
    ///   onnx_models/encoder_conv.onnx, encoder_transformer.onnx,
    ///   decoder_init.int8.onnx, decoder_step.int8.onnx, embed_tokens.bin
    ///   tokenizer.json
    pub fn new(model_dir: &Path) -> Result<Self, ArcanaError> {
        let onnx_dir = model_dir.join("onnx_models");

        // Проверка наличия всех обязательных файлов вынесена в check_required_files.
        let tokenizer_path = check_required_files(&onnx_dir, model_dir)?;

        tracing::info!("Загрузка Qwen3-ASR из: {:?}", model_dir);

        let n_threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);

        let load_session = |name: &str| -> Result<Session, ArcanaError> {
            #[allow(unused_mut)]
            let mut builder =
                Session::builder().map_err(|e| ArcanaError::ModelLoad(format!("Session builder: {}", e)))?;
            #[cfg(feature = "cuda")]
            {
                builder = builder
                    .with_execution_providers([CUDAExecutionProvider::default().build()])
                    .map_err(|e| ArcanaError::ModelLoad(format!("CUDA EP: {}", e)))?;
            }
            builder
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(|e| ArcanaError::ModelLoad(format!("Opt level: {}", e)))?
                .with_intra_threads(n_threads)
                .map_err(|e| ArcanaError::ModelLoad(format!("Threads: {}", e)))?
                .commit_from_file(onnx_dir.join(name))
                .map_err(|e| ArcanaError::ModelLoad(format!("Загрузка {}: {}", name, e)))
        };

        let encoder_conv = load_session("encoder_conv.onnx")?;
        let encoder_transformer = load_session("encoder_transformer.onnx")?;
        let decoder_init = load_session("decoder_init.int8.onnx")?;
        let decoder_step = load_session("decoder_step.int8.onnx")?;

        // Загрузка эмбеддингов [151936, 1024] float32 вынесена в load_embeddings.
        let embed_tokens = load_embeddings(&onnx_dir)?;

        // Токенизатор
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| ArcanaError::ModelLoad(format!("tokenizer.json: {}", e)))?;

        // Кэшируем IDs частых слов
        let encode_word = |word: &str| -> Vec<i64> {
            tokenizer
                .encode(word, false)
                .map(|enc| enc.get_ids().iter().map(|&id| id as i64).collect())
                .unwrap_or_default()
        };

        let system_ids = encode_word("system");
        let user_ids = encode_word("user");
        let assistant_ids = encode_word("assistant");

        tracing::info!(
            "Qwen3-ASR загружена: {} потоков, embed {}MB",
            n_threads,
            embed_tokens.len() * 4 / 1_000_000,
        );

        Ok(Self {
            encoder_conv: Mutex::new(encoder_conv),
            encoder_transformer: Mutex::new(encoder_transformer),
            decoder_init: Mutex::new(decoder_init),
            decoder_step: Mutex::new(decoder_step),
            embed_tokens,
            tokenizer,
            system_ids,
            user_ids,
            assistant_ids,
        })
    }

    /// Получить эмбеддинг токена по ID
    fn get_embedding(&self, token_id: i64) -> &[f32] {
        let idx = token_id as usize;
        &self.embed_tokens[idx * HIDDEN_SIZE..(idx + 1) * HIDDEN_SIZE]
    }

    /// Кодирование аудио: mel → conv → encoder → audio_features [N, 1024]
    fn encode_audio(&self, mel: &Array2<f32>) -> Result<Array2<f32>, ArcanaError> {
        let mel_len = mel.shape()[1];
        let chunk_num = mel_len.div_ceil(CHUNK_SIZE);

        // Собираем чанки и их длины
        let mut chunk_lengths = Vec::with_capacity(chunk_num);
        for i in 0..chunk_num {
            let start = i * CHUNK_SIZE;
            let end = (start + CHUNK_SIZE).min(mel_len);
            chunk_lengths.push(end - start);
        }

        let max_chunk_len = *chunk_lengths.iter().max().unwrap_or(&0);
        if max_chunk_len == 0 {
            return Ok(Array2::zeros((0, HIDDEN_SIZE)));
        }

        // Padded chunks: [chunk_num, 1, 128, max_chunk_len] (чистая упаковка).
        let padded = pack_padded_chunks(mel, &chunk_lengths, max_chunk_len);

        // Conv frontend
        let padded_contiguous = padded.as_standard_layout();
        let padded_tensor = TensorRef::from_array_view(padded_contiguous.view())
            .map_err(|e| ArcanaError::Recognizer(format!("Conv tensor: {}", e)))?;

        let mut conv_session = self
            .encoder_conv
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex: {}", e)))?;
        let conv_outputs = conv_session
            .run(ort::inputs!["padded_mel_chunks" => padded_tensor])
            .map_err(|e| ArcanaError::Recognizer(format!("Conv inference: {}", e)))?;

        let (conv_shape, conv_data) = conv_outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("Conv extract: {}", e)))?;

        // Распаковываем: убираем padding из каждого чанка (чистая распаковка).
        let conv_tokens_per_chunk = conv_shape[1] as usize;
        let conv_dim = conv_shape[2] as usize; // 896
        let lens_after_cnn: Vec<usize> = chunk_lengths
            .iter()
            .map(|&l| mel::feat_extract_output_lengths(l))
            .collect();

        let hidden_states = unpack_conv_output(conv_data, conv_tokens_per_chunk, conv_dim, &lens_after_cnn);
        let total_tokens = hidden_states.nrows();

        // Attention mask: [1, 1, total_tokens, total_tokens] — все нули (causal не нужен)
        let attn_mask = Array4::<f32>::zeros((1, 1, total_tokens, total_tokens));

        let hs_contiguous = hidden_states.as_standard_layout();
        let hs_tensor = TensorRef::from_array_view(hs_contiguous.view())
            .map_err(|e| ArcanaError::Recognizer(format!("HS tensor: {}", e)))?;
        let mask_contiguous = attn_mask.as_standard_layout();
        let mask_tensor = TensorRef::from_array_view(mask_contiguous.view())
            .map_err(|e| ArcanaError::Recognizer(format!("Mask tensor: {}", e)))?;

        let mut enc_session = self
            .encoder_transformer
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex: {}", e)))?;
        let enc_outputs = enc_session
            .run(ort::inputs![
                "hidden_states" => hs_tensor,
                "attention_mask" => mask_tensor
            ])
            .map_err(|e| ArcanaError::Recognizer(format!("Encoder inference: {}", e)))?;

        let (enc_shape, enc_data) = enc_outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("Encoder extract: {}", e)))?;

        let enc_tokens = enc_shape[0] as usize;
        let enc_dim = enc_shape[1] as usize; // 1024
        let audio_features = Array2::from_shape_vec((enc_tokens, enc_dim), enc_data.to_vec())
            .map_err(|e| ArcanaError::Recognizer(format!("Array reshape: {}", e)))?;

        Ok(audio_features)
    }

    /// Строит prompt token IDs с плейсхолдерами для аудио
    fn build_prompt_ids(&self, num_audio_tokens: usize) -> Vec<i64> {
        let mut ids = Vec::new();
        // <|im_start|>system\n<|im_end|>\n
        ids.push(IM_START_ID);
        ids.extend_from_slice(&self.system_ids);
        ids.push(NEWLINE_ID);
        ids.push(IM_END_ID);
        ids.push(NEWLINE_ID);
        // <|im_start|>user\n<|audio_start|><|audio_pad|>...<|audio_end|><|im_end|>\n
        ids.push(IM_START_ID);
        ids.extend_from_slice(&self.user_ids);
        ids.push(NEWLINE_ID);
        ids.push(AUDIO_START_ID);
        ids.extend(std::iter::repeat_n(AUDIO_PAD_ID, num_audio_tokens));
        ids.push(AUDIO_END_ID);
        ids.push(IM_END_ID);
        ids.push(NEWLINE_ID);
        // <|im_start|>assistant\n
        ids.push(IM_START_ID);
        ids.extend_from_slice(&self.assistant_ids);
        ids.push(NEWLINE_ID);

        ids
    }

    /// Embed tokens и заменяем audio_pad позиции на audio_features
    fn embed_and_fuse(&self, token_ids: &[i64], audio_features: &Array2<f32>) -> Vec<f32> {
        let seq_len = token_ids.len();
        let mut embeds = vec![0.0f32; seq_len * HIDDEN_SIZE];

        let mut audio_idx = 0;
        for (i, &tid) in token_ids.iter().enumerate() {
            if tid == AUDIO_PAD_ID && audio_idx < audio_features.nrows() {
                // Заменяем на audio feature
                let row = audio_features.row(audio_idx);
                embeds[i * HIDDEN_SIZE..(i + 1) * HIDDEN_SIZE].copy_from_slice(row.as_slice().unwrap());
                audio_idx += 1;
            } else {
                // Lookup embedding
                let emb = self.get_embedding(tid);
                embeds[i * HIDDEN_SIZE..(i + 1) * HIDDEN_SIZE].copy_from_slice(emb);
            }
        }

        embeds
    }

    /// Prefill-фаза: прогоняет `decoder_init` по эмбеддингам промпта, возвращает
    /// первый токен (argmax последнего кадра), начальный KV-cache и `vocab`.
    fn decoder_prefill(&self, seq_len: usize, embeds_flat: Vec<f32>) -> Result<(i64, KvCache, usize), ArcanaError> {
        // Reshape: [1, seq_len, 1024]
        let input_embeds = ndarray::Array3::from_shape_vec((1, seq_len, HIDDEN_SIZE), embeds_flat)
            .map_err(|e| ArcanaError::Recognizer(format!("Embeds reshape: {}", e)))?;
        let position_ids = ndarray::Array2::from_shape_vec((1, seq_len), (0..seq_len as i64).collect())
            .map_err(|e| ArcanaError::Recognizer(format!("Pos IDs: {}", e)))?;

        let embeds_contiguous = input_embeds.as_standard_layout();
        let embeds_tensor = TensorRef::from_array_view(embeds_contiguous.view())
            .map_err(|e| ArcanaError::Recognizer(format!("Embeds tensor: {}", e)))?;
        let pos_contiguous = position_ids.as_standard_layout();
        let pos_tensor = TensorRef::from_array_view(pos_contiguous.view())
            .map_err(|e| ArcanaError::Recognizer(format!("Pos tensor: {}", e)))?;

        let mut init_session = self
            .decoder_init
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex: {}", e)))?;
        let init_outputs = init_session
            .run(ort::inputs![
                "input_embeds" => embeds_tensor,
                "position_ids" => pos_tensor
            ])
            .map_err(|e| ArcanaError::Recognizer(format!("Decoder init: {}", e)))?;

        // Извлекаем logits → argmax последнего кадра
        let (logits_shape, logits_data) = init_outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("Logits extract: {}", e)))?;
        let vocab = logits_shape[2] as usize;
        let last_offset = (seq_len - 1) * vocab;
        let first_token = argmax_token(&logits_data[last_offset..last_offset + vocab]);

        // Извлекаем present_keys/values для KV cache
        let (_, keys_data) = init_outputs[1]
            .try_extract_tensor::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("Keys extract: {}", e)))?;
        let keys_shape: Vec<usize> = init_outputs[1]
            .try_extract_tensor::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("Keys shape: {}", e)))?
            .0
            .iter()
            .map(|&d| d as usize)
            .collect();
        let (_, values_data) = init_outputs[2]
            .try_extract_tensor::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("Values extract: {}", e)))?;

        Ok((
            first_token,
            KvCache {
                keys: keys_data.to_vec(),
                values: values_data.to_vec(),
                keys_shape,
            },
            vocab,
        ))
    }

    /// Один шаг авторегрессии: эмбеддит `last_token`, прогоняет `decoder_step` с
    /// текущим `kv`-cache, обновляет cache на месте и возвращает следующий токен.
    fn decoder_step_once(
        &self,
        last_token: i64,
        cur_pos: i64,
        kv: &mut KvCache,
        vocab: usize,
    ) -> Result<i64, ArcanaError> {
        // Embed одного токена: [1, 1, 1024]
        let emb = self.get_embedding(last_token);
        let token_embed = ndarray::Array3::from_shape_vec((1, 1, HIDDEN_SIZE), emb.to_vec())
            .map_err(|e| ArcanaError::Recognizer(format!("Token embed: {}", e)))?;
        let pos = ndarray::Array2::from_shape_vec((1, 1), vec![cur_pos])
            .map_err(|e| ArcanaError::Recognizer(format!("Step pos: {}", e)))?;

        // KV cache tensors (keys_shape общий для keys и values)
        let keys_nd = ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&kv.keys_shape), kv.keys.clone())
            .map_err(|e| ArcanaError::Recognizer(format!("Keys nd: {}", e)))?;
        let values_nd = ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&kv.keys_shape), kv.values.clone())
            .map_err(|e| ArcanaError::Recognizer(format!("Values nd: {}", e)))?;

        let te_contiguous = token_embed.as_standard_layout();
        let te_tensor = TensorRef::from_array_view(te_contiguous.view())
            .map_err(|e| ArcanaError::Recognizer(format!("TE tensor: {}", e)))?;
        let pos_c = pos.as_standard_layout();
        let pos_t = TensorRef::from_array_view(pos_c.view())
            .map_err(|e| ArcanaError::Recognizer(format!("Pos tensor: {}", e)))?;
        let keys_c = keys_nd.as_standard_layout();
        let keys_t = TensorRef::from_array_view(keys_c.view())
            .map_err(|e| ArcanaError::Recognizer(format!("Keys tensor: {}", e)))?;
        let values_c = values_nd.as_standard_layout();
        let values_t = TensorRef::from_array_view(values_c.view())
            .map_err(|e| ArcanaError::Recognizer(format!("Values tensor: {}", e)))?;

        let mut step_session = self
            .decoder_step
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex: {}", e)))?;
        let step_outputs = step_session
            .run(ort::inputs![
                "input_embeds" => te_tensor,
                "position_ids" => pos_t,
                "past_keys" => keys_t,
                "past_values" => values_t
            ])
            .map_err(|e| ArcanaError::Recognizer(format!("Decoder step: {}", e)))?;

        let (_, step_logits) = step_outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("Step logits: {}", e)))?;
        let next_token = argmax_token(&step_logits[..vocab]);

        // Обновляем KV cache на месте
        let (new_keys_shape, new_keys) = step_outputs[1]
            .try_extract_tensor::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("New keys: {}", e)))?;
        kv.keys_shape = new_keys_shape.iter().map(|&d| d as usize).collect();
        kv.keys = new_keys.to_vec();
        kv.values = step_outputs[2]
            .try_extract_tensor::<f32>()
            .map_err(|e| ArcanaError::Recognizer(format!("New values: {}", e)))?
            .1
            .to_vec();

        Ok(next_token)
    }
}

impl Transcriber for Qwen3AsrTranscriber {
    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> Result<String, ArcanaError> {
        // Общий препроцессинг: обрезка тишины + i16→f32 + resample до 16 кГц.
        let audio_f32 = preprocess_to_f32_16k(samples, sample_rate);
        if audio_f32.len() < 400 {
            return Ok(String::new());
        }

        // 1. Mel-спектрограмма [128, T]
        let mel_spec = mel::compute_mel_spectrogram(&audio_f32);
        if mel_spec.shape()[1] == 0 {
            return Ok(String::new());
        }

        // 2. Encoder: mel → audio_features [N, 1024]
        let audio_features = self.encode_audio(&mel_spec)?;
        let num_audio_tokens = audio_features.nrows();
        if num_audio_tokens == 0 {
            return Ok(String::new());
        }

        // 3. Build prompt и embed
        let token_ids = self.build_prompt_ids(num_audio_tokens);
        let seq_len = token_ids.len();
        let embeds_flat = self.embed_and_fuse(&token_ids, &audio_features);

        // 4. Decoder init (prefill): первый токен + начальный KV-cache + vocab.
        let (first_token, mut kv, vocab) = self.decoder_prefill(seq_len, embeds_flat)?;
        let mut generated = vec![first_token];

        // 5. Авторегрессивный цикл
        let max_new_tokens = 512;
        for (cur_pos, _) in (seq_len as i64..).zip(0..max_new_tokens - 1) {
            let last_token = *generated.last().unwrap();
            if last_token == IM_END_ID || last_token == ENDOFTEXT_ID {
                break;
            }
            let next_token = self.decoder_step_once(last_token, cur_pos, &mut kv, vocab)?;
            generated.push(next_token);
        }

        // Убираем EOS
        if let Some(&last) = generated.last()
            && (last == IM_END_ID || last == ENDOFTEXT_ID)
        {
            generated.pop();
        }

        // Декодируем токены в текст
        let token_ids_u32: Vec<u32> = generated.iter().map(|&id| id as u32).collect();
        let text = self
            .tokenizer
            .decode(&token_ids_u32, true)
            .map_err(|e| ArcanaError::Recognizer(format!("Decode: {}", e)))?;

        // Убираем служебные части (language tag, <asr_text>)
        let result = strip_asr_prefix(&text);

        Ok(result)
    }
}

// ONNX Session является Send + Sync через Mutex
unsafe impl Send for Qwen3AsrTranscriber {}
unsafe impl Sync for Qwen3AsrTranscriber {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_argmax_token() {
        assert_eq!(argmax_token(&[0.1, 0.9, 0.3]), 1);
        // Пустой slice → fallback ENDOFTEXT_ID.
        assert_eq!(argmax_token(&[]), ENDOFTEXT_ID);
        // NaN не должен «выигрывать»: argmax → индекс реального максимума.
        assert_eq!(argmax_token(&[f32::NAN, 0.5]), 1);
    }

    #[test]
    fn test_strip_asr_prefix() {
        assert_eq!(strip_asr_prefix("ru<asr_text>привет "), "привет");
        assert_eq!(strip_asr_prefix("нет тега"), "нет тега");
    }

    #[test]
    fn test_unpack_conv_output() {
        // 2 чанка по 3 токена × 2 dim в плоском буфере; берём 2 токена из chunk0,
        // 1 из chunk1 (остальное — отброшенный padding).
        let conv_data = [
            10.0, 11.0, 12.0, 13.0, 14.0, 15.0, // chunk0: t0,t1,t2
            20.0, 21.0, 22.0, 23.0, 24.0, 25.0, // chunk1: t0,t1,t2
        ];
        let out = unpack_conv_output(&conv_data, 3, 2, &[2, 1]);
        assert_eq!(out.shape(), &[3, 2]);
        assert_eq!(out.row(0).to_vec(), vec![10.0, 11.0]); // chunk0 t0
        assert_eq!(out.row(1).to_vec(), vec![12.0, 13.0]); // chunk0 t1
        assert_eq!(out.row(2).to_vec(), vec![20.0, 21.0]); // chunk1 t0
    }

    #[test]
    fn test_pack_padded_chunks() {
        // mel [128, 3]: задаём первую mel-строку, остальное нули.
        let mut mel = Array2::<f32>::zeros((N_MELS, 3));
        mel[[0, 0]] = 1.0;
        mel[[0, 1]] = 2.0;
        mel[[0, 2]] = 3.0;
        // chunk_lengths [2, 1], max 2 → форма [2, 1, 128, 2].
        let padded = pack_padded_chunks(&mel, &[2, 1], 2);
        assert_eq!(padded.shape(), &[2, 1, N_MELS, 2]);
        assert_eq!(padded[[0, 0, 0, 0]], 1.0); // chunk0 t0
        assert_eq!(padded[[0, 0, 0, 1]], 2.0); // chunk0 t1
        assert_eq!(padded[[1, 0, 0, 0]], 3.0); // chunk1 t0
        assert_eq!(padded[[1, 0, 0, 1]], 0.0); // chunk1 padding
    }
}
