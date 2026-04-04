#!/bin/bash

# --- НАСТРОЙКИ ---
KEYBINDING_NAME="ArcanaGlyph Trigger"
KEYBINDING_COMMAND="$HOME/.local/bin/ag-trigger" # Используем $HOME, это надежнее, чем $USER
KEYBINDING_SHORTCUT="<Control><Alt><Super>space" # Формат для gsettings

# --- ЛОГИКА СКРИПТА ---

# Путь к списку пользовательских сочетаний
CUSTOM_KEYBINDINGS_PATH="/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings"

# Получаем текущий список сочетаний
binding_list=$(gsettings get org.gnome.settings-daemon.plugins.media-keys custom-keybindings)
binding_list=${binding_list#*@as} # Убираем тип массива
binding_list=${binding_list//\'/} # Убираем одинарные кавычки

# Убираем [ и ] из начала и конца, если они есть
if [[ "$binding_list" == \[*\] ]]; then
	binding_list=${binding_list:1:-1}
fi

# Ищем свободный номер для нашего сочетания (custom0, custom1, ...)
n=0
while true; do
	if [[ ! $binding_list =~ custom$n ]]; then
		new_binding_path="$CUSTOM_KEYBINDINGS_PATH/custom$n/"
		break
	fi
	((n++))
done

echo "Найдено свободное место для сочетания: $new_binding_path"

# Добавляем наш новый путь в общий список
if [ -z "$binding_list" ]; then
	new_list="['$new_binding_path']"
else
	new_list="[$binding_list, '$new_binding_path']"
fi

# Устанавливаем обновленный список
gsettings set org.gnome.settings-daemon.plugins.media-keys custom-keybindings "$new_list"

# Настраиваем параметры нашего нового сочетания
gsettings set "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:$new_binding_path" name "$KEYBINDING_NAME"
gsettings set "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:$new_binding_path" command "$KEYBINDING_COMMAND"
gsettings set "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:$new_binding_path" binding "$KEYBINDING_SHORTCUT"

echo "Горячая клавиша '$KEYBINDING_SHORTCUT' для '$KEYBINDING_NAME' успешно добавлена!"
