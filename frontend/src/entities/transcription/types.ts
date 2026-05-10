// entities/transcription/types.ts
//
// История транскрибаций: запись с мета-данными + результат.

export interface HistoryEntry {
  recording: {
    id: number;
    audio_path: string;
    duration_secs: number;
    created_at: string;
  };
  transcription: {
    id: number;
    text: string;
    model: string;
    transcriber_type: string;
    created_at: string;
  };
  audio_exists: boolean;
}
