



# SwitchFix

**SwitchFix** — утилита в трее для KDE Plasma 6 (Wayland), которая исправляет текст, набранный не той раскладкой (EN↔RU), и может вычищать выделенное предложение через внешний LLM.

Аналог Punto Switcher / Keyboop для Kubuntu (Debian + KDE Plasma 6).

---

## Демо

Короткое видео (34 с): **[demo/switchfix-demo.mp4](demo/switchfix-demo.mp4)** — что происходит по хоткеям:

https://github.com/user-attachments/assets/61396501-9062-437a-9a1e-2c50097079aa

1. `Ctrl+Alt+R` — `ghbdtn vbh` → `привет мир`
2. `Alt+R` — LLM-чистка выделенного (трей мигает жёлтым)
3. `Backspace` — полная отмена замены
4. Шпаргалка по хоткеям

Это анимированный HTML-мокап (не живой скрин). Пересборка:

```bash
node tools/render-demo.mjs          # → demo/switchfix-demo.mp4
# demo/switchfix-demo.html — сцены и тайминг
```

---

## Возможности

| Функция | Как работает |
|---------|----------------|
| **Смена раскладки выделенного** | Выделите `ghbdtn` → `Ctrl+Alt+R` → получите `привет`. Системная раскладка переключается автоматически. В терминалах (konsole, kitty, …) — очистка строки + `Ctrl+Shift+V`. |
| **LLM-исправление выделенного** | Выделите предложение → `Alt+R` → текст уходит в OpenAI-совместимый API и заменяется результатом. **Одна `Backspace` в течение ~4 с полностью отменяет замену.** |
| **Tray** | Иконка с flash/звуком на хоткей, жёлтое мигание пока LLM думает, пауза/включить, настройки, выход. |
| **Виджет Plasma** | «SwitchFix LLM» — история запросов с копированием «было / стало» в один клик. |

Автозамен при наборе, подсказок, встроенного LLM и словарей **нет** — только явные действия по хоткею.

---

## Требования

- Linux, **KDE Plasma 6** (Wayland или X11)
- Группа **`input`** (для глобальных хоткеев и uinput)
- Опционально: OpenAI-совместимый API (OpenAI, Ollama, LM Studio, llama.cpp server, …)

```bash
sudo usermod -aG input "$USER"   # затем перелогиниться
```

---

## Установка (apt)

Debian / Ubuntu / Kubuntu — готовый deb-репозиторий на GitHub Pages:

```bash
echo "deb [trusted=yes] https://traineratwot.github.io/switchfix ./" \
  | sudo tee /etc/apt/sources.list.d/switchfix.list
sudo apt update
sudo apt install switchfix
```

Затем:

```bash
switchfix setup      # группа input + udev (sudo только если нужно)
switchfix doctor     # диагностика
systemctl --user enable --now switchfix.service   # автозапуск демона (по желанию)
```

Пакет ставит бинарник, карты раскладок, desktop-файл, systemd user unit, udev-правило и иконку. Страница репозитория: [traineratwot.github.io/switchfix](https://traineratwot.github.io/switchfix).

---

## Сборка и запуск

```bash
cargo build --release

./target/release/switchfix daemon     # демон в трее (основной режим)
./target/release/switchfix settings   # окно настроек
./target/release/switchfix doctor     # диагностика
./target/release/switchfix selftest   # проверка перехвата хоткеев
```

Установка (иконка, desktop-файл, systemd user unit, udev, виджет):

```bash
./packaging/install.sh
# или
make install
```

> systemd user service **не включается** автоматически. Включайте сами, если нужно:
> `systemctl --user enable --now switchfix.service`

---

## Горячие клавиши

| Действие | По умолчанию |
|----------|----------------|
| Сменить раскладку выделенного текста | `Ctrl+Alt+R` |
| LLM-исправить выделенный текст | `Alt+R` |
| Пауза / продолжить | `Ctrl+Alt+T` |
| **Отменить LLM-замену** | `Backspace` (в течение ~4 с после замены) |

Комбинации настраиваются в **Settings** (клик по полю → нажать клавиши; `Esc` — сбросить). Пустое поле = действие отключено.

Хоткеи можно «съедать» (`hotkey_consume`), чтобы они не доходили до других программ; обычная печать при этом работает.

---

## Настройка LLM

В Settings → **«LLM-исправление»** или в `~/.config/switchfix/config.toml`:

```toml
[llm]
enabled = true
base_url = "http://127.0.0.1:11434"   # корень OpenAI-совместимого API
api_token = ""                         # Bearer-токен; пусто = без Authorization
model = "qwen2.5:0.5b"                 # опционально
system_prompt = "Исправь орфографию… Верни ТОЛЬКО исправленный текст."
timeout_ms = 15000
```

Запрос идёт на `{base_url}/v1/chat/completions`.

Примеры `base_url`:

| Сервер | URL |
|--------|-----|
| OpenAI | `https://api.openai.com` |
| Ollama | `http://127.0.0.1:11434` |
| llama.cpp | `http://127.0.0.1:8080` |
| LM Studio | `http://127.0.0.1:1234` |

В Settings есть кнопки **«Тест»** (проверка API) и **«Модели»** (список моделей с автоподсказками — кликните, чтобы выбрать).

---

## Виджет Plasma

**SwitchFix LLM** — на панели: ПКМ → «Добавить виджеты» → *SwitchFix LLM*.

- История последних LLM-запросов (было → стало)
- Тексты укорочены под ширину виджета
- **«Копировать было» / «Копировать стало»** — в буфер обмена

История: `/tmp/switchfix.llm-history.json` (последние 30, до перезагрузки).

---

## CLI

| Команда | Назначение |
|---------|------------|
| `switchfix daemon` | Демон в трее (по умолчанию) |
| `switchfix settings` | Окно настроек |
| `switchfix convert --text 'ghbdtn'` | Конвертация без вставки (только печать) |
| `switchfix llm-history` | JSON истории LLM-запросов |
| `switchfix llm-copy --index 0 --field to` | Скопировать поле из истории |
| `switchfix doctor` | Диагностика (группы, uinput, D-Bus, хоткеи, LLM) |
| `switchfix setup` | Интерактивная настройка (input, udev, сервис) |
| `switchfix toggle` | Пауза / включить |
| `switchfix reload` | Перечитать config.toml и maps/ |
| `switchfix beep` | Проверка звука |
| `switchfix selftest` | Проверка перехвата хоткеев |

---

## Приватность

- Нет телеметрии и автозапросов в сеть.
- **LLM-исправление** (только по хоткею) отправляет **выделенный текст** и system-промпт на **вами указанный** API-адрес.
- Буфер набранных символов — только в RAM, опционален (`privacy.typing_buffer`).
- Токен API хранится в `config.toml` (рекомендуется `chmod 600`) и **не пишется в логи**.

Подробнее: [docs/PRIVACY.md](docs/PRIVACY.md), [docs/SECURITY.md](docs/SECURITY.md).

---

## Архитектура (кратко)

```
ui / daemon  →  app (use-cases, ports)  →  domain (pure)
                        ↑ implements
                   adapters (evdev, uinput, zbus, clipboard, OpenAI HTTP)
```

- **domain** — конвертация EN↔RU, модели (без I/O)
- **app** — `CorrectionService`, порты, конфиг, hotkey parse
- **adapters** — KDE D-Bus, evdev/uinput, clipboard, `OpenAiCorrector` (ureq)
- **ui** — tray (ksni), settings (iced), виджет Plasma
- **daemon** — wiring, CLI, doctor, setup

Тесты: `cargo test --workspace` (≈130 unit/integration).

---

## Документация для разработчиков

- [AGENTS.md](AGENTS.md) — архитектура, ports, landmines, FAQ
- [docs/PLAN.md](docs/PLAN.md) — план и roadmap
- [docs/MANUAL-TEST-MATRIX.md](docs/MANUAL-TEST-MATRIX.md) — ручная проверка

---

## Лицензия

MIT — см. `Cargo.toml`.
