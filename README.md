# ArcanaGlyph

Десктопное приложение для голосового ввода текста на Linux. Нажимаете горячую клавишу — говорите —
нажимаете ещё раз — распознанный текст автоматически вставляется в активное окно.
Вся транскрибация происходит локально через модель Vosk, без передачи данных в облако.

## Системные зависимости

```bash
sudo apt-get update && sudo apt-get install \
  build-essential \
  libasound2-dev \
  libgtk-3-dev \
  libwebkit2gtk-4.1-dev \
  libxdo-dev
```

Также необходима библиотека `libvosk`. Если она не установлена, можно собрать из исходников
скриптом `scripts/legacy/install_libvosk.bash`.

## Установка Vosk-модели

Скачайте русскую модель и распакуйте в директорию `models/`:

```bash
mkdir -p models
cd models
wget https://alphacephei.com/vosk/models/vosk-model-ru-0.42.zip
unzip vosk-model-ru-0.42.zip
cd ..
```

Либо укажите путь к модели в конфигурационном файле (см. раздел "Конфигурация").

## Запуск

```bash
# Одна команда — запускает приложение с иконкой в трее
make run
```

При первом запуске приложение создаст конфигурационный файл с настройками по умолчанию
в `~/.config/ArcanaGlyph/config.toml`.

## Горячие клавиши

По умолчанию: **Super+Alt+Control+Space**

- Первое нажатие — начинает запись с микрофона
- Второе нажатие — останавливает запись, транскрибирует и вставляет текст в активное окно
- Если не нажать повторно, запись автоматически остановится через 20 секунд

Горячую клавишу можно изменить в конфигурационном файле.

## Конфигурация

Файл: `~/.config/ArcanaGlyph/config.toml`

```toml
# Путь к Vosk-модели
model_path = "/home/user/projects/arcanaglyph/models/vosk-model-ru-0.42"

# Частота дискретизации аудио (Гц)
sample_rate = 48000

# Максимальное время записи (секунды)
max_record_secs = 20

# Автоматически вставлять текст в активное окно после транскрибации
auto_type = true

# Горячая клавиша (формат: модификаторы через + и клавиша)
hotkey = "Super+Alt+Control+Space"
```

### Параметры

| Параметр | Тип | По умолчанию | Описание |
| --- | --- | --- | --- |
| model_path | string | ./models/vosk-model-ru-0.42 | Путь к директории с Vosk-моделью |
| sample_rate | число | 48000 | Частота дискретизации микрофона |
| max_record_secs | число | 20 | Таймаут автоостановки записи |
| auto_type | bool | true | Вставлять текст в активное окно |
| hotkey | string | Super+Alt+Control+Space | Глобальная горячая клавиша |

## Сборка дистрибутива

```bash
# Создаёт .deb и .AppImage в target/release/bundle/
make dist
```

Требуется установленный Tauri CLI:

```bash
cargo install tauri-cli
```

## Разработка

```bash
make help     # Показать все доступные команды
make run      # Запустить приложение
make all      # Форматирование + линтинг + проверка + тесты
make fmt      # cargo fmt
make lint     # cargo clippy
make test     # cargo test
make check    # cargo check
make build    # Release-сборка
make clean    # Очистка кэша
```

## Структура проекта

```text
crates/
  arcanaglyph-core/    # Библиотека: движок (Vosk + cpal + enigo)
  arcanaglyph-app/     # Tauri v2 приложение (GUI + tray + горячие клавиши)
dist/
  index.html           # Фронтенд (vanilla HTML/JS)
models/
  vosk-model-ru-0.42/  # Vosk-модель (не в git)
```

## Troubleshooting

### Ошибка "unable to find library -lvosk"

Библиотека `libvosk.so` не найдена линкером. Добавьте путь:

```bash
export LIBRARY_PATH=/usr/local/lib
```

Или установите libvosk через `scripts/legacy/install_libvosk.bash`.

### Горячая клавиша не работает

На Wayland глобальные горячие клавиши могут не работать из-за ограничений протокола.
Попробуйте запустить в сессии X11 или используйте кнопку в окне приложения.

### Модель не найдена

Убедитесь, что путь к модели корректен в `~/.config/ArcanaGlyph/config.toml`.
Путь должен указывать на директорию, содержащую файлы `am/`, `graph/`, `conf/` и другие.

### enigo: Ошибка "Не удалось создать Enigo"

Установите `libxdo-dev`:

```bash
sudo apt-get install libxdo-dev
```
