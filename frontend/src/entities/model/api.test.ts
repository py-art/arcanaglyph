import { describe, expect, it } from 'vitest';
import { getModels, getCompiledEngines, downloadModel, deleteModel } from './api';

const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
  mockResolvedValue: (v: unknown) => void;
  mock: { calls: unknown[][] };
};

describe('entities/model/api', () => {
  it('getModels возвращает список или [] при null', async () => {
    invokeMock.mockResolvedValue([{ id: 'm1' }]);
    expect(await getModels()).toEqual([{ id: 'm1' }]);
    invokeMock.mockResolvedValue(null);
    expect(await getModels()).toEqual([]);
  });

  it('getCompiledEngines возвращает список или [] при null', async () => {
    invokeMock.mockResolvedValue(['whisper', 'gigaam']);
    expect(await getCompiledEngines()).toEqual(['whisper', 'gigaam']);
    invokeMock.mockResolvedValue(null);
    expect(await getCompiledEngines()).toEqual([]);
  });

  it('downloadModel зовёт команду download_model с аргументами', async () => {
    invokeMock.mockResolvedValue(undefined);
    await downloadModel('m1', 'http://u', '/dest');
    expect(invokeMock.mock.calls.at(-1)).toEqual(['download_model', { modelId: 'm1', url: 'http://u', destDir: '/dest' }]);
  });

  it('deleteModel зовёт команду delete_model с аргументами', async () => {
    invokeMock.mockResolvedValue(undefined);
    await deleteModel('m1', '/path');
    expect(invokeMock.mock.calls.at(-1)).toEqual(['delete_model', { modelId: 'm1', path: '/path' }]);
  });
});
