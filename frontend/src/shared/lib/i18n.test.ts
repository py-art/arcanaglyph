import { describe, expect, it } from 'vitest';
import { t, i18n } from './i18n';

describe('i18n.t', () => {
  it('делегирует в window.i18n.t без переменных', () => {
    expect(t('status.ready')).toBe('status.ready');
  });

  it('пробрасывает переменные подстановки', () => {
    expect(t('history.page', { current: 1, total: 3 })).toBe('history.page:{"current":1,"total":3}');
  });

  it('re-export i18n указывает на тот же объект window.i18n', () => {
    expect(i18n.getLanguage()).toBe('ru');
  });
});
