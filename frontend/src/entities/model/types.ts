// entities/model/types.ts
//
// Domain-типы для модели транскрибации. Зеркало
// `arcanaglyph_core::transcription_models`.

import type { TranscriberType } from '../config/types';

export interface Model {
  id: string;
  transcriber_type: TranscriberType;
  display_name: string;
  size_mb?: number;
  download_url?: string;
  available: boolean;
  active?: boolean;
}

/** Карта коротких display-имён для model badge на главной странице. */
export const MODEL_SHORT_NAMES: Record<string, string> = {
  'vosk-model-ru-0.42': 'Vosk',
  'ggml-large-v3-turbo.bin': 'Whisper Large',
  'ggml-tiny.bin': 'Whisper Tiny',
  'gigaam-v3-e2e-ctc': 'GigaAM v3',
  'qwen3-asr-0.6b': 'Qwen3-ASR',
};
