// crates/arcanaglyph-core/src/qwen3asr/transcriber/encode.rs
//
// Кодирование аудио Qwen3-ASR: mel → conv-frontend → encoder → audio_features.

use ndarray::{Array2, Array4};
use ort::value::TensorRef;

use crate::error::ArcanaError;
use crate::qwen3asr::mel;

use super::{CHUNK_SIZE, HIDDEN_SIZE, N_MELS, Qwen3AsrTranscriber, recog};

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

impl Qwen3AsrTranscriber {
    /// Кодирование аудио: mel → conv → encoder → audio_features [N, 1024]
    pub(super) fn encode_audio(&self, mel: &Array2<f32>) -> Result<Array2<f32>, ArcanaError> {
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
        let padded_tensor = recog(TensorRef::from_array_view(padded_contiguous.view()), "Conv tensor")?;

        let mut conv_session = self
            .encoder_conv
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex: {}", e)))?;
        let conv_outputs = recog(
            conv_session.run(ort::inputs!["padded_mel_chunks" => padded_tensor]),
            "Conv inference",
        )?;

        let (conv_shape, conv_data) = recog(conv_outputs[0].try_extract_tensor::<f32>(), "Conv extract")?;

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
        let hs_tensor = recog(TensorRef::from_array_view(hs_contiguous.view()), "HS tensor")?;
        let mask_contiguous = attn_mask.as_standard_layout();
        let mask_tensor = recog(TensorRef::from_array_view(mask_contiguous.view()), "Mask tensor")?;

        let mut enc_session = self
            .encoder_transformer
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex: {}", e)))?;
        let enc_outputs = recog(
            enc_session.run(ort::inputs![
                "hidden_states" => hs_tensor,
                "attention_mask" => mask_tensor
            ]),
            "Encoder inference",
        )?;

        let (enc_shape, enc_data) = recog(enc_outputs[0].try_extract_tensor::<f32>(), "Encoder extract")?;

        let enc_tokens = enc_shape[0] as usize;
        let enc_dim = enc_shape[1] as usize; // 1024
        let audio_features = recog(
            Array2::from_shape_vec((enc_tokens, enc_dim), enc_data.to_vec()),
            "Array reshape",
        )?;

        Ok(audio_features)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
