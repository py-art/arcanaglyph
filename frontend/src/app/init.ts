// app/init.ts
//
// Главный entry приложения: монтирует все widgets / features / pages.
// После завершения FSD-миграции (Phase 6-11) main.ts стал thin entry —
// весь UI поднимается отсюда.

import { mountTitlebar } from '../widgets/titlebar/titlebar';
import { mountModelBadge, updateModelBadge } from '../widgets/model-badge/model-badge';
import { mountMainControls } from '../widgets/main-controls/main-controls';
import { mountPortalBanner } from '../widgets/portal-banner/portal-banner';
import { mountUpdateBanner } from '../widgets/update-banner/update-banner';
import { mountAboutPage } from '../pages/about/about';
import { initPageNavigation, subscribePage } from '../features/page-navigation/page-navigation';
import { mountSettings } from '../features/settings/settings';
import { mountHistoryPage } from '../pages/history/history-page';

/**
 * Инициализация UI. Порядок имеет значение: titlebar → controls → badge →
 * banners (могут полагаться на toast/i18n из shared) → about (зависит
 * от updateBanner.window.__showUpdateBanner для manual triggera) →
 * page-navigation (нужны DOM-узлы settings/history/about на момент
 * подписки) → features (settings / history) которые вешают обработчики
 * на DOM этих страниц.
 *
 * Возвращает API для совместимости с тонким main.ts (на случай если
 * понадобится из консоли разработчика дёрнуть onModelReady вручную).
 */
export function initApp(): { onModelReady: () => void } {
  mountTitlebar();
  mountModelBadge();
  const main = mountMainControls();
  mountUpdateBanner();
  void mountPortalBanner();
  void mountAboutPage();

  // Page-navigation должна быть инициализирована ДО mountSettings/mountHistoryPage
  // (они подписываются на subscribePage и зовут showPage в menuBtn-handler'ах).
  initPageNavigation();

  // При возврате на главную страницу — обновить badge движка (на settings
  // юзер мог сменить движок и сохранить, badge должен подтянуть актуальный).
  // Раньше эту логику включал showPage-reassignment в main.ts.
  subscribePage(page => {
    if (page === 'main') void updateModelBadge();
  });

  mountSettings();
  mountHistoryPage();

  return main;
}
