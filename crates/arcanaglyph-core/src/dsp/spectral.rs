// crates/arcanaglyph-core/src/dsp/spectral.rs
//
// Спектральные DSP-примитивы для препроцессинга mel-спектрограмм.
// Единственный источник истины для оконной функции и STFT-ядра, которые
// раньше дублировались в gigaam/mel.rs и qwen3asr/mel.rs.
//
// Что здесь: оконо Ханна + power-STFT (с опциональным reflect-паддингом).
// Чего здесь НЕТ: mel-filterbank (HTK у GigaAM vs Slaney у Qwen3) и
// log/нормализация — это «характер модели», остаётся в каждом бэкенде.

use std::borrow::Cow;

use ndarray::Array2;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

/// Окно Ханна (periodic, как `librosa`/`torch.hann_window(periodic=True)`).
/// `w[i] = 0.5 * (1 - cos(2*pi*i/size))`. Граничное `w[0] == 0`.
pub fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| {
            let phase = 2.0 * std::f32::consts::PI * i as f32 / size as f32;
            0.5 * (1.0 - phase.cos())
        })
        .collect()
}

/// Параметры STFT.
///
/// `center` управляет reflect-паддингом сигнала на `n_fft/2` сэмплов с каждой
/// стороны (librosa `center=True`). При `center=false` паддинга нет.
/// `win_length` сейчас всегда равен `n_fft` (окно той же длины, что и кадр FFT).
pub struct StftConfig {
    pub n_fft: usize,
    pub hop_length: usize,
    pub win_length: usize,
    pub center: bool,
}

/// Power-спектр STFT в КАНОНИЧЕСКОМ layout `[n_bins, n_frames]`,
/// где `n_bins = n_fft/2 + 1`. Возвращает `|X|²`.
///
/// Порядок операций: (опц. reflect-pad) → оконо → `rustfft` forward → `norm_sqr`.
/// Если после паддинга сигнал короче `n_fft`, возвращает `Array2::zeros((n_bins, 0))`.
///
/// `window` должен иметь длину `n_fft` (см. `win_length`).
pub fn stft_power(signal: &[f32], window: &[f32], cfg: &StftConfig) -> Array2<f32> {
    let n_bins = cfg.n_fft / 2 + 1;

    // Опциональный reflect-паддинг (center=true как в librosa/Whisper).
    let padded: Cow<'_, [f32]> = if cfg.center {
        Cow::Owned(reflect_pad(signal, cfg.n_fft / 2))
    } else {
        Cow::Borrowed(signal)
    };

    if padded.len() < cfg.n_fft {
        return Array2::zeros((n_bins, 0));
    }

    let n_frames = (padded.len() - cfg.n_fft) / cfg.hop_length + 1;

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(cfg.n_fft);

    let mut result = Array2::zeros((n_bins, n_frames));
    let mut buffer = vec![Complex::new(0.0f32, 0.0f32); cfg.n_fft];

    for frame_idx in 0..n_frames {
        let start = frame_idx * cfg.hop_length;

        // Применяем окно и копируем кадр в FFT-буфер.
        for (i, buf) in buffer.iter_mut().enumerate() {
            *buf = Complex::new(padded[start + i] * window[i], 0.0);
        }

        fft.process(&mut buffer);

        // Power spectrum: |X|².
        for bin in 0..n_bins {
            result[[bin, frame_idx]] = buffer[bin].norm_sqr();
        }
    }

    result
}

/// Reflect-паддинг сигнала на `pad` сэмплов с каждой стороны
/// (librosa-совместимый: индексы зеркалятся, не включая саму границу).
///
/// Guard'ы (`.min(len-1)` спереди, `saturating_sub` сзади) перенесены из
/// исходного qwen3-кода точь-в-точь — защита от паники на сигнале короче `pad`.
/// Предполагает `signal.len() >= 1` (как и исходный код).
pub(crate) fn reflect_pad(signal: &[f32], pad: usize) -> Vec<f32> {
    let len = signal.len();
    let mut padded = Vec::with_capacity(len + 2 * pad);

    // Reflect в начале.
    for i in (1..=pad).rev() {
        let idx = i.min(len - 1);
        padded.push(signal[idx]);
    }
    padded.extend_from_slice(signal);
    // Reflect в конце.
    for i in 1..=pad {
        let idx = len.saturating_sub(1 + i);
        padded.push(signal[idx]);
    }

    padded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hann_window_320_properties() {
        let w = hann_window(320);
        assert_eq!(w.len(), 320);
        // Periodic window: w[0] == 0.
        assert!(w[0].abs() < 1e-6);
        // Все значения в [0, 1].
        assert!(w.iter().all(|&v| (0.0..=1.0).contains(&v)));
        // Максимум около середины.
        let max_idx = w
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert!((max_idx as i32 - 160).abs() <= 1, "макс на {max_idx}, ждали ~160");
    }

    #[test]
    fn test_hann_window_400_symmetry() {
        let w = hann_window(400);
        assert_eq!(w.len(), 400);
        assert!(w[0].abs() < 1e-6);
        // Симметрия: w[i] ≈ w[400 - i] для i из 1..200.
        for i in 1..200 {
            assert!((w[i] - w[400 - i]).abs() < 1e-6, "несимметрия на {i}");
        }
    }

    #[test]
    fn test_stft_power_shape_no_center() {
        let signal = vec![0.0f32; 16000];
        let window = hann_window(320);
        let cfg = StftConfig {
            n_fft: 320,
            hop_length: 160,
            win_length: 320,
            center: false,
        };
        let p = stft_power(&signal, &window, &cfg);
        // n_bins = 161, T = (16000 - 320) / 160 + 1 = 99.
        assert_eq!(p.shape(), &[161, 99]);
    }

    #[test]
    fn test_stft_power_shape_center() {
        let signal = vec![0.0f32; 16000];
        let window = hann_window(400);
        let cfg = StftConfig {
            n_fft: 400,
            hop_length: 160,
            win_length: 400,
            center: true,
        };
        let p = stft_power(&signal, &window, &cfg);
        // n_bins = 201, T = (16000 + 400 - 400) / 160 + 1 = 101.
        assert_eq!(p.shape(), &[201, 101]);
    }

    #[test]
    fn test_stft_power_short_input() {
        let signal = vec![0.0f32; 100];
        let window = hann_window(320);
        let cfg = StftConfig {
            n_fft: 320,
            hop_length: 160,
            win_length: 320,
            center: false,
        };
        let p = stft_power(&signal, &window, &cfg);
        assert_eq!(p.shape(), &[161, 0]);
    }

    #[test]
    fn test_stft_power_sine_peak_bin() {
        // Синус 1 кГц @ 16 кГц: пик power в бине ≈ 1000 * 320 / 16000 = 20.
        let signal: Vec<f32> = (0..16000)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 16000.0).sin())
            .collect();
        let window = hann_window(320);
        let cfg = StftConfig {
            n_fft: 320,
            hop_length: 160,
            win_length: 320,
            center: false,
        };
        let p = stft_power(&signal, &window, &cfg);
        let mid = p.ncols() / 2;
        let mut max_bin = 0;
        let mut max_val = f32::NEG_INFINITY;
        for bin in 0..p.nrows() {
            let v = p[[bin, mid]];
            if v > max_val {
                max_val = v;
                max_bin = bin;
            }
        }
        assert!((max_bin as i32 - 20).abs() <= 2, "пик в бине {max_bin}, ждали ~20");
    }

    #[test]
    fn test_reflect_pad_basic() {
        let out = reflect_pad(&[1.0, 2.0, 3.0, 4.0], 2);
        assert_eq!(out, vec![3.0, 2.0, 1.0, 2.0, 3.0, 4.0, 3.0, 2.0]);
    }

    #[test]
    fn test_reflect_pad_short_no_panic() {
        // Сигнал длиной 1 короче pad — guard'ы не должны паниковать.
        let out = reflect_pad(&[7.0], 2);
        assert_eq!(out.len(), 1 + 2 * 2);
    }
}
