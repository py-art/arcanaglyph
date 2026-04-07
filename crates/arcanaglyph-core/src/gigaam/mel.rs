// crates/arcanaglyph-core/src/gigaam/mel.rs
//
// Вычисление mel-спектрограммы для GigaAM v3.
// Параметры: 16kHz, 64 mel bins, n_fft=320, hop=160, HTK scale, center=false.

use ndarray::{Array2, Array3};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Параметры препроцессинга GigaAM v3
const SAMPLE_RATE: u32 = 16000;
const N_FFT: usize = 320;
const HOP_LENGTH: usize = 160;
const WIN_LENGTH: usize = 320;
const N_MELS: usize = 64;

/// Вычисляет log-mel спектрограмму для GigaAM v3.
/// Вход: f32 сэмплы при 16kHz. Выход: [1, 64, T_frames].
pub fn compute_mel_spectrogram(samples: &[f32]) -> Array3<f32> {
    if samples.len() < WIN_LENGTH {
        // Слишком короткое аудио — возвращаем пустую спектрограмму
        return Array3::zeros((1, N_MELS, 0));
    }

    let window = hann_window(WIN_LENGTH);
    let power_spec = stft(samples, &window);
    let filterbank = mel_filterbank();

    let n_frames = power_spec.nrows();
    let mut mel_spec = Array2::zeros((N_MELS, n_frames));

    // mel_spec = filterbank @ power_spec.T
    for frame in 0..n_frames {
        for mel in 0..N_MELS {
            let mut sum = 0.0f32;
            for bin in 0..filterbank.ncols() {
                sum += filterbank[[mel, bin]] * power_spec[[frame, bin]];
            }
            // log(clamp(x, 1e-9, 1e9))
            mel_spec[[mel, frame]] = sum.clamp(1e-9, 1e9).ln();
        }
    }

    mel_spec.insert_axis(ndarray::Axis(0))
}

/// Окно Ханна (Hann window)
fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| {
            let phase = 2.0 * std::f32::consts::PI * i as f32 / size as f32;
            0.5 * (1.0 - phase.cos())
        })
        .collect()
}

/// STFT без center-паддинга.
/// Возвращает power spectrum: [n_frames, n_fft/2 + 1]
fn stft(signal: &[f32], window: &[f32]) -> Array2<f32> {
    let n_bins = N_FFT / 2 + 1; // 161
    let n_frames = (signal.len() - WIN_LENGTH) / HOP_LENGTH + 1;

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);

    let mut result = Array2::zeros((n_frames, n_bins));
    let mut buffer = vec![Complex::new(0.0f32, 0.0f32); N_FFT];

    for frame_idx in 0..n_frames {
        let start = frame_idx * HOP_LENGTH;

        // Применяем окно и копируем в FFT-буфер
        for (i, buf) in buffer.iter_mut().enumerate() {
            *buf = Complex::new(signal[start + i] * window[i], 0.0);
        }

        fft.process(&mut buffer);

        // Power spectrum: |X|²
        for bin in 0..n_bins {
            result[[frame_idx, bin]] = buffer[bin].norm_sqr();
        }
    }

    result
}

/// HTK mel filterbank: [n_mels, n_fft/2 + 1]
fn mel_filterbank() -> Array2<f32> {
    let n_bins = N_FFT / 2 + 1; // 161

    // Границы mel-шкалы
    let mel_low = hz_to_mel(0.0);
    let mel_high = hz_to_mel(SAMPLE_RATE as f32 / 2.0);

    // Равномерно распределённые точки в mel-шкале (n_mels + 2 точки)
    let mel_points: Vec<f32> = (0..=N_MELS + 1)
        .map(|i| mel_low + (mel_high - mel_low) * i as f32 / (N_MELS + 1) as f32)
        .collect();

    // Конвертируем обратно в Hz и затем в FFT-бины
    let hz_points: Vec<f32> = mel_points.iter().map(|&m| mel_to_hz(m)).collect();
    let bin_points: Vec<f32> = hz_points
        .iter()
        .map(|&f| f * N_FFT as f32 / SAMPLE_RATE as f32)
        .collect();

    let mut filterbank = Array2::zeros((N_MELS, n_bins));

    for mel in 0..N_MELS {
        let left = bin_points[mel];
        let center = bin_points[mel + 1];
        let right = bin_points[mel + 2];

        for bin in 0..n_bins {
            let freq_bin = bin as f32;

            if freq_bin >= left && freq_bin <= center && center > left {
                // Восходящий склон
                filterbank[[mel, bin]] = (freq_bin - left) / (center - left);
            } else if freq_bin > center && freq_bin <= right && right > center {
                // Нисходящий склон
                filterbank[[mel, bin]] = (right - freq_bin) / (right - center);
            }
        }
    }

    filterbank
}

/// Hz → mel (HTK формула)
fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

/// mel → Hz (HTK формула)
fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0f32.powf(mel / 2595.0) - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hann_window_properties() {
        let w = hann_window(320);
        assert_eq!(w.len(), 320);
        // Граничные значения близки к 0 (periodic window: w[0]=0, w[N] ≈ 0)
        assert!(w[0].abs() < 1e-6);
        // Все значения в [0, 1]
        assert!(w.iter().all(|&v| v >= 0.0 && v <= 1.0));
        // Максимум около середины
        let max_idx = w.iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap().0;
        assert!((max_idx as i32 - 160).abs() <= 1, "Максимум на позиции {}, ожидалось ~160", max_idx);
    }

    #[test]
    fn test_mel_filterbank_shape() {
        let fb = mel_filterbank();
        assert_eq!(fb.shape(), &[64, 161]);
        // Все значения >= 0
        assert!(fb.iter().all(|&v| v >= 0.0));
        // Каждый фильтр имеет хотя бы один ненулевой элемент
        for mel in 0..64 {
            let row_sum: f32 = fb.row(mel).iter().sum();
            assert!(row_sum > 0.0, "Фильтр {} пустой", mel);
        }
    }

    #[test]
    fn test_hz_mel_roundtrip() {
        for &hz in &[0.0, 440.0, 1000.0, 4000.0, 8000.0] {
            let restored = mel_to_hz(hz_to_mel(hz));
            assert!((hz - restored).abs() < 0.01, "hz={} restored={}", hz, restored);
        }
    }

    #[test]
    fn test_mel_spectrogram_output_shape() {
        // 1 секунда аудио при 16kHz = 16000 сэмплов
        let samples = vec![0.0f32; 16000];
        let spec = compute_mel_spectrogram(&samples);
        // T_frames = (16000 - 320) / 160 + 1 = 99
        assert_eq!(spec.shape(), &[1, 64, 99]);
    }

    #[test]
    fn test_mel_spectrogram_short_audio() {
        // Аудио короче одного окна
        let samples = vec![0.0f32; 100];
        let spec = compute_mel_spectrogram(&samples);
        assert_eq!(spec.shape(), &[1, 64, 0]);
    }

    #[test]
    fn test_mel_spectrogram_sine_wave() {
        // Синусоида 1kHz — энергия должна быть сконцентрирована в средних mel-бинах
        let samples: Vec<f32> = (0..16000)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 16000.0).sin())
            .collect();
        let spec = compute_mel_spectrogram(&samples);
        assert_eq!(spec.shape()[0], 1);
        assert_eq!(spec.shape()[1], 64);
        assert!(spec.shape()[2] > 0);

        // Средний кадр: максимальная энергия не в первом и не в последнем mel-бине
        let mid_frame = spec.shape()[2] / 2;
        let mut max_mel = 0;
        let mut max_val = f32::NEG_INFINITY;
        for mel in 0..64 {
            let val = spec[[0, mel, mid_frame]];
            if val > max_val {
                max_val = val;
                max_mel = mel;
            }
        }
        // 1kHz должна быть примерно в 15-25 mel-бине (не на краях)
        assert!(max_mel > 5 && max_mel < 50, "1kHz в mel-бине {}, ожидалось 10-40", max_mel);
    }
}
