// entities/recording/types.ts
//
// События engine, которые приходят через `listen('engine://*')` в UI.
// Зеркало `arcanaglyph_core::EngineEvent`.

export type EngineEventName =
  | 'engine://recording-started'
  | 'engine://recording-stopped'
  | 'engine://transcription-result'
  | 'engine://error'
  | 'engine://model-loading'
  | 'engine://model-loaded'
  | 'engine://request-focus'
  | 'engine://finished-processing';

export interface TranscriptionResult {
  text: string;
}

export interface EngineError {
  message: string;
}

export interface ModelLoadingPayload {
  model: string;
}
