# ArcanaGlyph

## Run

```bash
# запускаем сервер в одном терминале
meke serv
# запускаем приложение в другом терминале
make run
# отправляем триггер в третьем терминале
echo "trigger" | nc -u -w0 127.0.0.1 9002
# либо нажимаем горячие клавиши Ctrl+Win+Alt+Space
```

## Short Cut

```bash
# Путь к настройкам горячих клавиш в GNOME
GSETTINGS_PATH="/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings"

# Создаем новую запись для нашего хоткея
gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings \
"['$GSETTINGS_PATH/custom0/']"

# Настраиваем нашу новую запись
gsettings set $GSETTINGS_PATH/custom0/ name 'ArcanaGlyph Trigger'
gsettings set $GSETTINGS_PATH/custom0/ command '/home/py-art/.local/bin/ag-trigger'
gsettings set $GSETTINGS_PATH/custom0/ binding 'Ctrl+Space'
```
