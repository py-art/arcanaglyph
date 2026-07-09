// crates/arcanaglyph-core/src/qwen3asr/transcriber/load.rs
//
// Загрузка модели Qwen3-ASR: проверка файлов, эмбеддинги, конструктор.

use std::path::Path;
use std::sync::Mutex;

#[cfg(feature = "cuda")]
use ort::execution_providers::CUDAExecutionProvider;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;

use crate::error::ArcanaError;

use super::{HIDDEN_SIZE, Qwen3AsrTranscriber, VOCAB_SIZE};

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

/// Загружает одну ONNX-сессию `name` из `onnx_dir` с оптимизацией Level3 и
/// `n_threads` intra-потоками (при feature `cuda` — с CUDA execution provider).
/// Вынесено из `new`, чтобы убрать замыкание с cfg-веткой из конструктора.
fn load_onnx_session(onnx_dir: &Path, name: &str, n_threads: usize) -> Result<Session, ArcanaError> {
    #[allow(unused_mut)]
    let mut builder = Session::builder().map_err(|e| ArcanaError::ModelLoad(format!("Session builder: {}", e)))?;
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

        let encoder_conv = load_onnx_session(&onnx_dir, "encoder_conv.onnx", n_threads)?;
        let encoder_transformer = load_onnx_session(&onnx_dir, "encoder_transformer.onnx", n_threads)?;
        let decoder_init = load_onnx_session(&onnx_dir, "decoder_init.int8.onnx", n_threads)?;
        let decoder_step = load_onnx_session(&onnx_dir, "decoder_step.int8.onnx", n_threads)?;

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
}
