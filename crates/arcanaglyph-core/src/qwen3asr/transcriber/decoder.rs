// crates/arcanaglyph-core/src/qwen3asr/transcriber/decoder.rs
//
// Авторегрессивный декодер Qwen3-ASR: эмбеддинги промпта, prefill и пошаговая
// генерация токенов с KV-cache.

use ndarray::Array2;
use ort::value::TensorRef;

use crate::error::ArcanaError;

use super::{
    AUDIO_END_ID, AUDIO_PAD_ID, AUDIO_START_ID, ENDOFTEXT_ID, HIDDEN_SIZE, IM_END_ID, IM_START_ID, NEWLINE_ID,
    Qwen3AsrTranscriber, recog,
};

/// Greedy argmax по slice логитов → token id. Пустой slice или NaN-only →
/// fallback `ENDOFTEXT_ID`. Дедуплицирует argmax из prefill и шага декодера.
/// Чистая функция (тестируется без ONNX).
fn argmax_token(logits: &[f32]) -> i64 {
    crate::decode::argmax_index(logits)
        .map(|idx| idx as i64)
        .unwrap_or(ENDOFTEXT_ID)
}

/// KV-cache декодера, прокидываемый между шагами авторегрессии. `keys_shape`
/// используется и для keys, и для values (как в исходном коде).
pub(super) struct KvCache {
    keys: Vec<f32>,
    values: Vec<f32>,
    keys_shape: Vec<usize>,
}

impl Qwen3AsrTranscriber {
    /// Получить эмбеддинг токена по ID
    fn get_embedding(&self, token_id: i64) -> &[f32] {
        let idx = token_id as usize;
        &self.embed_tokens[idx * HIDDEN_SIZE..(idx + 1) * HIDDEN_SIZE]
    }

    /// Строит prompt token IDs с плейсхолдерами для аудио
    pub(super) fn build_prompt_ids(&self, num_audio_tokens: usize) -> Vec<i64> {
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
    pub(super) fn embed_and_fuse(
        &self,
        token_ids: &[i64],
        audio_features: &Array2<f32>,
    ) -> Result<Vec<f32>, ArcanaError> {
        let seq_len = token_ids.len();
        let mut embeds = vec![0.0f32; seq_len * HIDDEN_SIZE];

        let mut audio_idx = 0;
        for (i, &tid) in token_ids.iter().enumerate() {
            if tid == AUDIO_PAD_ID && audio_idx < audio_features.nrows() {
                // Заменяем на audio feature. Для стандартного row-major Array2 срез contiguous;
                // при non-contiguous view возвращаем ошибку распознавания вместо паники.
                let row = audio_features.row(audio_idx);
                let row_slice = row.as_slice().ok_or_else(|| {
                    ArcanaError::Recognizer("audio feature row не contiguous (non-standard ndarray layout)".to_string())
                })?;
                embeds[i * HIDDEN_SIZE..(i + 1) * HIDDEN_SIZE].copy_from_slice(row_slice);
                audio_idx += 1;
            } else {
                // Lookup embedding
                let emb = self.get_embedding(tid);
                embeds[i * HIDDEN_SIZE..(i + 1) * HIDDEN_SIZE].copy_from_slice(emb);
            }
        }

        Ok(embeds)
    }

    /// Prefill-фаза: прогоняет `decoder_init` по эмбеддингам промпта, возвращает
    /// первый токен (argmax последнего кадра), начальный KV-cache и `vocab`.
    pub(super) fn decoder_prefill(
        &self,
        seq_len: usize,
        embeds_flat: Vec<f32>,
    ) -> Result<(i64, KvCache, usize), ArcanaError> {
        // Reshape: [1, seq_len, 1024]
        let input_embeds = recog(
            ndarray::Array3::from_shape_vec((1, seq_len, HIDDEN_SIZE), embeds_flat),
            "Embeds reshape",
        )?;
        let position_ids = recog(
            ndarray::Array2::from_shape_vec((1, seq_len), (0..seq_len as i64).collect()),
            "Pos IDs",
        )?;

        let embeds_contiguous = input_embeds.as_standard_layout();
        let embeds_tensor = recog(TensorRef::from_array_view(embeds_contiguous.view()), "Embeds tensor")?;
        let pos_contiguous = position_ids.as_standard_layout();
        let pos_tensor = recog(TensorRef::from_array_view(pos_contiguous.view()), "Pos tensor")?;

        let mut init_session = self
            .decoder_init
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex: {}", e)))?;
        let init_outputs = recog(
            init_session.run(ort::inputs![
                "input_embeds" => embeds_tensor,
                "position_ids" => pos_tensor
            ]),
            "Decoder init",
        )?;

        // Извлекаем logits → argmax последнего кадра
        let (logits_shape, logits_data) = recog(init_outputs[0].try_extract_tensor::<f32>(), "Logits extract")?;
        let vocab = logits_shape[2] as usize;
        let last_offset = (seq_len - 1) * vocab;
        let first_token = argmax_token(&logits_data[last_offset..last_offset + vocab]);

        // Извлекаем present_keys/values для KV cache
        let (_, keys_data) = recog(init_outputs[1].try_extract_tensor::<f32>(), "Keys extract")?;
        let keys_shape: Vec<usize> = recog(init_outputs[1].try_extract_tensor::<f32>(), "Keys shape")?
            .0
            .iter()
            .map(|&d| d as usize)
            .collect();
        let (_, values_data) = recog(init_outputs[2].try_extract_tensor::<f32>(), "Values extract")?;

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
    pub(super) fn decoder_step_once(
        &self,
        last_token: i64,
        cur_pos: i64,
        kv: &mut KvCache,
        vocab: usize,
    ) -> Result<i64, ArcanaError> {
        // Embed одного токена: [1, 1, 1024]
        let emb = self.get_embedding(last_token);
        let token_embed = recog(
            ndarray::Array3::from_shape_vec((1, 1, HIDDEN_SIZE), emb.to_vec()),
            "Token embed",
        )?;
        let pos = recog(ndarray::Array2::from_shape_vec((1, 1), vec![cur_pos]), "Step pos")?;

        // KV cache tensors (keys_shape общий для keys и values)
        let keys_nd = recog(
            ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&kv.keys_shape), kv.keys.clone()),
            "Keys nd",
        )?;
        let values_nd = recog(
            ndarray::ArrayD::from_shape_vec(ndarray::IxDyn(&kv.keys_shape), kv.values.clone()),
            "Values nd",
        )?;

        let te_contiguous = token_embed.as_standard_layout();
        let te_tensor = recog(TensorRef::from_array_view(te_contiguous.view()), "TE tensor")?;
        let pos_c = pos.as_standard_layout();
        let pos_t = recog(TensorRef::from_array_view(pos_c.view()), "Pos tensor")?;
        let keys_c = keys_nd.as_standard_layout();
        let keys_t = recog(TensorRef::from_array_view(keys_c.view()), "Keys tensor")?;
        let values_c = values_nd.as_standard_layout();
        let values_t = recog(TensorRef::from_array_view(values_c.view()), "Values tensor")?;

        let mut step_session = self
            .decoder_step
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex: {}", e)))?;
        let step_outputs = recog(
            step_session.run(ort::inputs![
                "input_embeds" => te_tensor,
                "position_ids" => pos_t,
                "past_keys" => keys_t,
                "past_values" => values_t
            ]),
            "Decoder step",
        )?;

        let (_, step_logits) = recog(step_outputs[0].try_extract_tensor::<f32>(), "Step logits")?;
        let next_token = argmax_token(&step_logits[..vocab]);

        // Обновляем KV cache на месте
        let (new_keys_shape, new_keys) = recog(step_outputs[1].try_extract_tensor::<f32>(), "New keys")?;
        kv.keys_shape = new_keys_shape.iter().map(|&d| d as usize).collect();
        kv.keys = new_keys.to_vec();
        kv.values = recog(step_outputs[2].try_extract_tensor::<f32>(), "New values")?
            .1
            .to_vec();

        Ok(next_token)
    }
}

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
}
