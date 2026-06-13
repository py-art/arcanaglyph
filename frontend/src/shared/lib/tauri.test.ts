import { describe, expect, it } from 'vitest';
import { isApiError, errorMessage, errorHint, isCancelled, tryInvoke, type ApiError } from './tauri';

const err: ApiError = { kind: 'modelLoad', message: 'не загрузилось', hint: 'скачайте модель' };

describe('isApiError', () => {
  it('распознаёт корректный ApiError-payload', () => {
    expect(isApiError(err)).toBe(true);
  });

  it('отвергает не-объекты и объекты без обязательных полей', () => {
    expect(isApiError(null)).toBe(false);
    expect(isApiError('строка')).toBe(false);
    expect(isApiError({ kind: 'x' })).toBe(false); // нет message
    expect(isApiError({ kind: 'x', message: 42 })).toBe(false); // message не string
  });
});

describe('errorMessage', () => {
  it('берёт message из ApiError', () => {
    expect(errorMessage(err)).toBe('не загрузилось');
  });

  it('разворачивает Error и приводит прочее через String', () => {
    expect(errorMessage(new Error('boom'))).toBe('boom');
    expect(errorMessage(123)).toBe('123');
  });
});

describe('errorHint', () => {
  it('возвращает hint для ApiError и undefined для остального', () => {
    expect(errorHint(err)).toBe('скачайте модель');
    expect(errorHint({ kind: 'internal', message: 'x' })).toBeUndefined();
    expect(errorHint('строка')).toBeUndefined();
  });
});

describe('isCancelled', () => {
  it('true только для kind=cancelled', () => {
    expect(isCancelled({ kind: 'cancelled', message: 'x' })).toBe(true);
    expect(isCancelled(err)).toBe(false);
    expect(isCancelled(null)).toBe(false);
  });
});

describe('tryInvoke', () => {
  const invokeMock = (globalThis as Record<string, unknown>).__invokeMock as {
    mockResolvedValue: (v: unknown) => void;
    mockRejectedValue: (v: unknown) => void;
  };

  it('возвращает результат invoke при успехе', async () => {
    invokeMock.mockResolvedValue('ok');
    expect(await tryInvoke('cmd')).toBe('ok');
  });

  it('возвращает null при ошибке (не пробрасывает)', async () => {
    invokeMock.mockRejectedValue(new Error('нет команды'));
    expect(await tryInvoke('cmd')).toBeNull();
  });
});
