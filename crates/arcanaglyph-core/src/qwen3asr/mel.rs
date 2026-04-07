// crates/arcanaglyph-core/src/qwen3asr/mel.rs
//
// Mel-спектрограмма для Qwen3-ASR (Whisper-совместимая).
// Параметры: 16kHz, 128 mel bins, n_fft=400, hop=160, Slaney norm, center=true.

use ndarray::{Array1, Array2};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

const SAMPLE_RATE: u32 = 16000;
const N_FFT: usize = 400;
const HOP_LENGTH: usize = 160;
const N_MELS: usize = 128;

/// Вычисляет Whisper-совместимую log-mel спектрограмму.
/// Вход: f32 сэмплы при 16kHz. Выход: [128, T_frames].
pub fn compute_mel_spectrogram(samples: &[f32]) -> Array2<f32> {
    if samples.len() < N_FFT {
        return Array2::zeros((N_MELS, 0));
    }

    let window = hann_window(N_FFT);
    let filterbank = mel_filterbank_slaney();

    // Center padding (reflect)
    let pad = N_FFT / 2;
    let mut padded = Vec::with_capacity(samples.len() + 2 * pad);
    // Reflect padding в начале
    for i in (1..=pad).rev() {
        let idx = i.min(samples.len() - 1);
        padded.push(samples[idx]);
    }
    padded.extend_from_slice(samples);
    // Reflect padding в конце
    for i in 1..=pad {
        let idx = samples.len().saturating_sub(1 + i);
        padded.push(samples[idx]);
    }

    // STFT
    let n_bins = N_FFT / 2 + 1;
    let n_frames = (padded.len() - N_FFT) / HOP_LENGTH + 1;

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);

    let mut power_spec = Array2::zeros((n_bins, n_frames));
    let mut buffer = vec![Complex::new(0.0f32, 0.0f32); N_FFT];

    for frame_idx in 0..n_frames {
        let start = frame_idx * HOP_LENGTH;
        for (i, buf) in buffer.iter_mut().enumerate() {
            *buf = Complex::new(padded[start + i] * window[i], 0.0);
        }
        fft.process(&mut buffer);
        for bin in 0..n_bins {
            power_spec[[bin, frame_idx]] = buffer[bin].norm_sqr();
        }
    }

    // Mel filterbank → log scale (Whisper-style)
    let mut mel_spec = filterbank.dot(&power_spec);

    // log10(max(mel, 1e-10))
    mel_spec.mapv_inplace(|x| x.max(1e-10).log10());

    // Dynamic range: max(x, max - 8.0)
    let max_val = mel_spec.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let min_val = max_val - 8.0;
    mel_spec.mapv_inplace(|x| x.max(min_val));

    // Нормализация: (x + 4.0) / 4.0
    mel_spec.mapv_inplace(|x| (x + 4.0) / 4.0);

    mel_spec
}

/// Hann window (periodic)
fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| {
            let phase = 2.0 * std::f32::consts::PI * i as f32 / size as f32;
            0.5 * (1.0 - phase.cos())
        })
        .collect()
}

/// Slaney mel filterbank: [128, 201]
/// fmin=0, fmax=8000, norm="slaney", htk=false
fn mel_filterbank_slaney() -> Array2<f32> {
    let n_bins = N_FFT / 2 + 1; // 201
    let fmin = 0.0f32;
    let fmax = (SAMPLE_RATE / 2) as f32; // 8000

    // Slaney (non-HTK) mel scale
    let f_sp = 200.0 / 3.0;
    let min_log_hz = 1000.0f32;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = (6.4f32).ln() / 27.0;

    let hz_to_mel = |hz: f32| -> f32 {
        if hz < min_log_hz {
            hz / f_sp
        } else {
            min_log_mel + (hz / min_log_hz).ln() / logstep
        }
    };

    let mel_to_hz = |mel: f32| -> f32 {
        if mel < min_log_mel {
            mel * f_sp
        } else {
            min_log_hz * ((mel - min_log_mel) * logstep).exp()
        }
    };

    let mel_min = hz_to_mel(fmin);
    let mel_max = hz_to_mel(fmax);

    // n_mels + 2 точки
    let mel_points: Vec<f32> = (0..=N_MELS + 1)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (N_MELS + 1) as f32)
        .collect();

    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();

    // FFT bin frequencies
    let fft_freqs: Array1<f32> = Array1::from_vec(
        (0..n_bins)
            .map(|i| i as f32 * SAMPLE_RATE as f32 / N_FFT as f32)
            .collect(),
    );

    let mut filterbank = Array2::zeros((N_MELS, n_bins));

    for mel in 0..N_MELS {
        let left = hz_points[mel];
        let center = hz_points[mel + 1];
        let right = hz_points[mel + 2];

        for bin in 0..n_bins {
            let freq = fft_freqs[bin];
            if freq >= left && freq <= center && center > left {
                filterbank[[mel, bin]] = (freq - left) / (center - left);
            } else if freq > center && freq <= right && right > center {
                filterbank[[mel, bin]] = (right - freq) / (right - center);
            }
        }

        // Slaney normalization: divide by bandwidth
        let enorm = 2.0 / (hz_points[mel + 2] - hz_points[mel]);
        for bin in 0..n_bins {
            filterbank[[mel, bin]] *= enorm;
        }
    }

    filterbank
}

/// Вычисляет длину выхода после 3x stride-2 свёрток
pub fn feat_extract_output_lengths(input_length: usize) -> usize {
    let mut l = input_length;
    for _ in 0..3 {
        l = (l - 1) / 2 + 1;
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mel_output_shape() {
        // 1 секунда при 16kHz
        let samples = vec![0.0f32; 16000];
        let mel = compute_mel_spectrogram(&samples);
        assert_eq!(mel.shape()[0], 128);
        // n_frames = (16000 + 400 - 400) / 160 + 1 = 101
        assert_eq!(mel.shape()[1], 101);
    }

    #[test]
    fn test_feat_output_lengths() {
        assert_eq!(feat_extract_output_lengths(100), 13);
        assert_eq!(feat_extract_output_lengths(50), 7);
    }

    #[test]
    fn test_slaney_filterbank_shape() {
        let fb = mel_filterbank_slaney();
        assert_eq!(fb.shape(), &[128, 201]);
        // Все значения >= 0
        assert!(fb.iter().all(|&v| v >= 0.0));
    }
}
