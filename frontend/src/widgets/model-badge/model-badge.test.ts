import { beforeEach, describe, expect, it } from 'vitest';
import { updateModelBadge, mountModelBadge } from './model-badge';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockResolvedValue: (v: unknown) => void;
  mockRejectedValue: (v: unknown) => void;
};

// Хелпер внутри внешнего describe (см. hotkey-config.test) — top-level function
// declaration ломает синтез test_scope-узла графа → потеря test→prod рёбер.
describe('model-badge', () => {
  function buildBadge(): HTMLElement {
    const el = document.createElement('div');
    el.id = 'model-badge';
    document.body.appendChild(el);
    return el;
  }

  beforeEach(buildBadge);

  it('updateModelBadge маппит filename активной модели в короткое имя', async () => {
    mountModelBadge();
    invokeMock.mockResolvedValue({ transcriber: 'vosk', model_path: '/m/vosk-model-ru-0.42' });
    await updateModelBadge();
    expect(document.getElementById('model-badge')!.textContent).toBe('Vosk');
  });

  it('updateModelBadge для неизвестного filename показывает сам filename', async () => {
    mountModelBadge();
    invokeMock.mockResolvedValue({ transcriber: 'gigaam', gigaam_model_path: '/m/custom-model' });
    await updateModelBadge();
    expect(document.getElementById('model-badge')!.textContent).toBe('custom-model');
  });

  it('updateModelBadge тихо игнорирует ошибку backend', async () => {
    mountModelBadge();
    invokeMock.mockRejectedValue(new Error('нет engine'));
    await expect(updateModelBadge()).resolves.toBeUndefined();
  });

  it('updateModelBadge без badge в DOM — no-op без падения', async () => {
    document.body.innerHTML = '';
    await expect(updateModelBadge()).resolves.toBeUndefined();
  });
});
