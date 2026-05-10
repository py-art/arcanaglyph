// app/init.ts
//
// Главный entry приложения: монтирует всё что вынесено в FSD-слои
// и зовёт inline-инициализацию для блоков, которые ещё не извлечены
// (settings, model-management, history — TODO следующей итерации).

import { mountTitlebar } from '../widgets/titlebar/titlebar';
import { mountModelBadge } from '../widgets/model-badge/model-badge';
import { mountMainControls } from '../widgets/main-controls/main-controls';
import { mountPortalBanner } from '../widgets/portal-banner/portal-banner';
import { mountUpdateBanner } from '../widgets/update-banner/update-banner';
import { mountAboutPage } from '../pages/about/about';
// NOTE: features/page-navigation/ — feature-stub. Не подключён, потому что
// history-блок в main.ts делает `showPage = function(...)` reassignment;
// import-binding ESM-модуля immutable. Включить после рефакторинга
// history (заменить reassignment на subscribe pattern).

/**
 * Инициализация UI. Порядок имеет значение: titlebar → controls → badge →
 * banners (могут полагаться на toast/i18n из shared) → about (зависит
 * от updateBanner.window.__showUpdateBanner для manual triggera).
 *
 * Возвращает API для inline-блоков main.ts, которые ещё не extracted
 * в FSD-слои (settings/history/models). Они временно держат `onModelReady`
 * чтобы дёргать его из своих listener'ов.
 */
export function initApp(): { onModelReady: () => void } {
  mountTitlebar();
  mountModelBadge();
  const main = mountMainControls();
  mountUpdateBanner();
  void mountPortalBanner();
  void mountAboutPage();
  return main;
}
