// extension/arcanaglyph-widget@arfi.tech/extension.js
//
// Позиционирует плавающий виджет ArcanaGlyph на GNOME Wayland.
//
// Зачем: на Wayland mutter игнорирует приложенческое позиционирование окна
// (xdg_toplevel security model). Расширение работает внутри процесса gnome-shell
// и имеет доступ к Meta.Window API, через который перемещение окон легально.
//
// Идентификация виджета: по `title === "ArcanaGlyph Recording Widget"` (его
// устанавливает приложение в WebviewWindowBuilder). Совпадение по wm_class не
// уникально — wm_class "arcanaglyph" общий с главным окном приложения.
//
// Источник позиции: gsettings ключ
// `org.gnome.shell.extensions.arcanaglyph-widget position` (string из 9 значений
// "top-left" / ... / "bottom-right"). Приложение пишет его в save_config.
// Расширение слушает изменения и репозиционирует уже открытое окно мгновенно.

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const WIDGET_TITLE = 'ArcanaGlyph Recording Widget';
const MARGIN = 24;
const TOP_OFFSET = 48;
const BOTTOM_OFFSET = 60;

export default class ArcanaGlyphWidgetExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._mapHandlerId = global.window_manager.connect('map', (_wm, actor) => {
            this._maybePosition(actor.meta_window);
        });
        this._settingsHandlerId = this._settings.connect('changed::position', () => {
            // При смене позиции через UI приложения — двигаем уже открытый виджет.
            this._forEachWidget(win => this._positionWindow(win));
        });
        // Если расширение включили после старта приложения — пройдёмся по уже
        // открытым окнам и применим позицию.
        this._forEachWidget(win => this._positionWindow(win));
    }

    disable() {
        if (this._mapHandlerId) {
            global.window_manager.disconnect(this._mapHandlerId);
            this._mapHandlerId = null;
        }
        if (this._settingsHandlerId && this._settings) {
            this._settings.disconnect(this._settingsHandlerId);
            this._settingsHandlerId = null;
        }
        this._settings = null;
    }

    _maybePosition(win) {
        if (!win || win.get_title() !== WIDGET_TITLE) return;
        // Окно map'ится сразу но frame_rect ещё может быть нулевым;
        // отложим ~50 мс чтобы получить актуальный размер виджета.
        // setTimeout доступен в GJS начиная с GNOME 45.
        setTimeout(() => this._positionWindow(win), 50);
    }

    _forEachWidget(fn) {
        global.get_window_actors().forEach(actor => {
            const win = actor.meta_window;
            if (win && win.get_title() === WIDGET_TITLE) fn(win);
        });
    }

    _positionWindow(win) {
        if (!this._settings) return;
        const pos = this._settings.get_string('position') || 'bottom-center';
        const monitor = win.get_monitor();
        const workArea = win.get_work_area_for_monitor(monitor);
        const frame = win.get_frame_rect();
        const widgetW = frame.width;
        const widgetH = frame.height;

        const xLeft = workArea.x + MARGIN;
        const xCenter = workArea.x + Math.round((workArea.width - widgetW) / 2);
        const xRight = workArea.x + workArea.width - widgetW - MARGIN;
        const yTop = workArea.y + TOP_OFFSET;
        const yMid = workArea.y + Math.round((workArea.height - widgetH) / 2);
        const yBot = workArea.y + workArea.height - widgetH - BOTTOM_OFFSET;

        const map = {
            'top-left':      [xLeft,   yTop],
            'top-center':    [xCenter, yTop],
            'top-right':     [xRight,  yTop],
            'middle-left':   [xLeft,   yMid],
            'middle-center': [xCenter, yMid],
            'middle-right':  [xRight,  yMid],
            'bottom-left':   [xLeft,   yBot],
            'bottom-center': [xCenter, yBot],
            'bottom-right':  [xRight,  yBot],
        };
        const [x, y] = map[pos] || map['bottom-center'];
        win.move_frame(true, x, y);
    }
}
