import { describe, expect, it } from 'vitest';
import { mountMainControls } from './main-controls';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockResolvedValue: (v: unknown) => void;
};

describe('mountMainControls', () => {
  const IDS = [
    'status', 'result-wrap', 'result', 'copy-btn', 'mic-btn', 'mic-glow',
    'timer', 'level-bar', 'level-fill', 'controls', 'stop-btn', 'pause-btn',
  ];

  function buildDom(): void {
    for (const id of IDS) {
      const el = document.createElement('div');
      el.id = id;
      document.body.appendChild(el);
    }
  }

  it('возвращает onModelReady; вызов переводит статус в «готов»', () => {
    buildDom();
    // is_model_loaded → true: poll-цикл выходит сразу, без зависания.
    invokeMock.mockResolvedValue(true);
    const api = mountMainControls();
    expect(typeof api.onModelReady).toBe('function');
    api.onModelReady();
    expect(document.getElementById('status')!.textContent).toBe('status.ready');
    expect((document.getElementById('mic-btn') as HTMLElement).style.cursor).toBe('pointer');
  });
});
