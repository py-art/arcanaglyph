// crates/arcanaglyph-core/src/decode.rs
//
// Общие примитивы декодирования выхода STT-сетей (логиты → индекс токена), не
// привязанные к конкретному движку. Нужны только «нейросетевым» бэкендам
// (GigaAM CTC/RNN-T, Qwen3-ASR); при whisper/vosk-only сборке модуль не
// компилируется (иначе dead_code под clippy -D).

/// Индекс максимума в плоском срезе логитов (greedy argmax).
///
/// NaN-безопасно: несравнимые значения трактуются как `Equal` (та же идиома,
/// что во всех движках-потребителях). Возвращает `None` только для пустого
/// среза — вызывающая сторона подставляет свой fallback (blank / EOS / 0).
/// Чистая функция (тестируется без ONNX).
pub(crate) fn argmax_index(logits: &[f32]) -> Option<usize> {
    logits
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argmax_index_picks_max() {
        assert_eq!(argmax_index(&[0.1, 0.5, 0.2]), Some(1));
    }

    #[test]
    fn argmax_index_empty_is_none() {
        assert_eq!(argmax_index(&[]), None);
    }
}
