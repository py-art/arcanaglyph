// crates/arcanaglyph-core/src/qwen3asr/transcriber/mod.rs
//
// Транскрайбер Qwen3-ASR-0.6B (ONNX, мультиязычный, авторегрессивный decoder).
// 4 ONNX-файла: encoder_conv, encoder_transformer, decoder_init, decoder_step
// + embed_tokens.bin (матрица эмбеддингов) + tokenizer.json (BPE)
//
// Модуль разбит на подмодули по концернам:
//   load.rs    — проверка файлов, эмбеддинги, конструктор `new`
//   encode.rs  — кодирование аудио (mel → conv → encoder)
//   decoder.rs — авторегрессивный декодер (prefill + пошаговая генерация, KV-cache)
// Здесь остаётся структура, общие helper'ы и оркестрация `Transcriber::transcribe`.

use std::sync::Mutex;

use ort::session::Session;

use crate::dsp::preprocess_to_f32_16k;
use crate::error::ArcanaError;
use crate::transcriber::Transcriber;

use super::mel;

mod decoder;
mod encode;
mod load;

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

/// Вырезает служебный префикс (language tag) до и включая `<asr_text>` и тримит.
/// Если тега нет — просто тримит. Чистая функция.
fn strip_asr_prefix(decoded: &str) -> String {
    if let Some(idx) = decoded.find("<asr_text>") {
        decoded[idx + 10..].trim().to_string()
    } else {
        decoded.trim().to_string()
    }
}

/// Оборачивает ошибку любого источника в `ArcanaError::Recognizer` с контекстом.
/// Убирает повторяющиеся `.map_err(|e| ...)`-замыкания на пути авторегрессии
/// декодера (строка остаётся `"<ctx>: <e>"` — поведение не меняется).
fn recog<T, E: std::fmt::Display>(res: Result<T, E>, ctx: &str) -> Result<T, ArcanaError> {
    res.map_err(|e| ArcanaError::Recognizer(format!("{}: {}", ctx, e)))
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
        let embeds_flat = self.embed_and_fuse(&token_ids, &audio_features)?;

        // 4. Decoder init (prefill): первый токен + начальный KV-cache + vocab.
        let (first_token, mut kv, vocab) = self.decoder_prefill(seq_len, embeds_flat)?;
        let mut generated = vec![first_token];

        // 5. Авторегрессивный цикл
        let max_new_tokens = 512;
        for (cur_pos, _) in (seq_len as i64..).zip(0..max_new_tokens - 1) {
            // generated инициализирован непустым (first_token); let-else — защита от паники
            // на случай пустого вектора: пусто → генерировать больше нечего, выходим.
            let Some(&last_token) = generated.last() else {
                break;
            };
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
    fn test_strip_asr_prefix() {
        assert_eq!(strip_asr_prefix("ru<asr_text>привет "), "привет");
        assert_eq!(strip_asr_prefix("нет тега"), "нет тега");
    }
}
