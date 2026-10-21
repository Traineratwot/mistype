# Mistype — алгоритмы работы

Документ описывает, **как именно** утилита выполняет свои действия: захват выделения,
замена текста, переключение раскладки, перехват хоткеев, LLM-исправление и отмена.

Код: `crates/mistype-app/src/services/correction.rs` (оркестрация),
`crates/mistype-domain/src/conversion.rs` (конвертация),
`crates/mistype-adapters/src/platform/` (ОС-адаптеры).

---

## 1. Замена текста (конвертация раскладки)

**Триггер:** `Ctrl+Alt+R` (по умолчанию) — «исправить выделенное».

### 1.1 Общая схема

```
hotkey → CorrectionService::handle(ConvertSelection)
  → проверки (enabled, excluded_apps, fullscreen, terminal?)
  → захват выделения (clipboard-marker protocol)
  → конвертация символов (EN↔RU по таблицам)
  → замена выделения (clipboard + Ctrl+V)
  → восстановление пользовательского буфера
  → переключение системной раскладки (best-effort)
  → Idle
```

### 1.2 Захват выделения (clipboard-marker protocol)

Проблема: в Wayland/X11 нельзя прочитать выделение напрямую (AT-SPI не используется).
Решение — **marker-протокол** через буфер обмена:

1. Сохранить текущий буфер пользователя (`SavedClipboard`).
2. Положить в буфер **уникальный маркер** `MISTYPE_MARK_<uuid>`.
3. Отправить «копировать» в фокусированное окно, по очереди:
   - `Ctrl+Insert` (безопасно в терминалах),
   - `Ctrl+Shift+C` (терминалы),
   - `Ctrl+C` (GUI).
   
   Никогда не начинаем с `Ctrl+C` — в Konsole это SIGINT.
4. После каждого аккорда подождать `copy_settle_ms/2` и прочитать буфер.
5. Если содержимое **отличается от маркера** и непустое — это и есть выделение.
6. Если маркер пережил все три аккорда — **выделения нет** → `NoInputFound`.
   Откат к «последнему слову» через Backspace **запрещён**: одна Backspace
   удаляет всё выделение (баг «selection vanished»).

> Один инстанс `ArboardClipboard` обслуживает и `ClipboardPort`, и `SelectionCapture`
> (два `Arc`-вида одного объекта) — иначе ломается `last_save` маркер-протокола.

### 1.3 Конвертация (детект направления + таблица)

Психологически: пользователь набрал `ghbdtn` в русской раскладке, ожидая `привет`.

1. `detect_direction(text)` — считает долю латиницы (`a-z`) и кириллицы
   (`U+0400..U+04FF`) в тексте:
   - латиницы больше → `(us → ru)`,
   - кириллицы больше → `(ru → us)`,
   - поровну (и есть буквы) → `(us → ru)` (конвенция wordwrap),
   - букв нет → `None` (нечего конвертировать).
2. `convert(text, tables, from, to)` — построчная замена символов по таблице
   `ConversionTable` (`HashMap<char, char>`). Символы без маппинга остаются как есть.
3. Если результат == исходный текст → `NoInputFound` (нечего менять).

Таблицы загружаются из `~/.config/mistype/maps/en2ru.json` / `ru2en.json`
(формат `MapFile`); при отсутствии — встроенная таблица QWERTY ↔ ЙЦУКЕН
(`en_to_ru_map()`).

### 1.4 Замена выделения

1. `put_text(converted)` — положить результат в буфер.
2. **Проверка offer:** `get_text()` — если буфер не содержит результат
   (offer «испарился» — типичная проблема Wayland), **не** шлём `Ctrl+V`
   (он вставит пустоту и сотрёт выделение), а `type_text(converted)` —
   первый символ заменяет выделение.
3. Иначе `Ctrl+V`.
4. `sleep(paste_settle_ms)` — дать приложению употребить paste.
5. Восстановить пользовательский буфер (best-effort, опция `restore_clipboard`).

**Терминал** (авто-детект по `behavior.terminal_apps`): вместо `Ctrl+V` —
`Ctrl+A`, `Ctrl+K` (очистка строки) + `Ctrl+Shift+V` (paste).

### 1.5 Переключение раскладки (после конвертации)

См. §2. Лучший effort — **текст не откатываем**, если раскладка не переключилась.

---

## 2. Изменение системной раскладки

**Зачем:** после конвертации `ghbdtn` → `привет` пользователь продолжит печатать
в целевой раскладке. Если не переключить — следующий символ снова будет «не той».

### 2.1 KDE Plasma 6 (D-Bus)

Адаптер: `platform/linux/kde/layout.rs` → `KdeLayoutSwitcher`.

| Шаг | Действие |
|-----|----------|
| 1 | D-Bus session: сервис `org.kde.keyboard`, путь `/Layouts`, интерфейс `org.kde.KeyboardLayouts` |
| 2 | `getLayoutsList()` → список `(short_id, ?, display_name)` |
| 3 | `getLayout()` → текущий 0-based индекс (кэшируется) |
| 4 | Определить целевой `short_id` по `detect_direction(результат)` — «та сторона», куда конвертировали |
| 5 | Найти индекс целевого layout в списке |
| 6 | `setLayout(index)` |
| 7 | Poll `getLayout()` до совпадения (~50 ms) — Plasma 6 переключает асинхронно |

Сигнал `layoutChanged(u)` подписывается на внешние переключения (Meta+Space,
UI KDE) — кэш обновляется, daemon не рассинхронизируется.

> **Важно:** в zbus имена методов Plasma 6 **camelCase** — нужно явное
> `#[zbus(name = "getLayout")]`, иначе збус шлёт `GetLayout` и получает ошибку.

### 2.2 Порядок и best-effort

```
switch_layout_for(converted):
  target = detect_direction(converted)   // "та сторона", куда конвертировали
  layouts = layouts()                    // ошибка → warning «раскладка не переключилась»
  info = find(layouts, target)           // нет в списке KDE → тихо skip
  set(info.index)                        // ошибка → warning
```

**Правило:** ошибка layout-свитча **никогда не откатывает** уже вставленный текст.
Уведомление: «Текст исправлен, но раскладка не переключилась».

---

## 3. Перехват хоткеев (evdev + uinput)

**Зачем так:** на Wayland глобальные хоткеи через X11 API не работают; portal
`GlobalShortcuts` не поддерживает произвольные chords. Берём низкий уровень —
`/dev/input/event*` (нужна группа `input`).

### 3.1 Наблюдение

1. `EvdevHotkeySource` открывает все клавиатурные устройства (`evdev`).
2. `set_nonblocking(true)` на каждом — иначе первый device «съедает» цикл.
3. Цикл `fetch_events()`, при `WouldBlock` — skip.
4. Сопоставление с `HotkeyBinding` (из `config.toml`, парсится `HotkeyBinding::parse`).
5. Совпало → `hotkey received: …` в лог + в канал `HotkeySource::events()`.

### 3.2 Потребление (hotkey_consume, default true)

1. Сначала открывается **passthrough uinput** (виртуальная клавиатура).
2. Только потом `EVIOCGRAB` на реальных устройствах.
3. Все события, кроме хоткея, инжектятся обратно через passthrough.
4. **Если passthrough упал — grab НЕ выполняется** (иначе клавиатура умрёт системно).
5. Устройства с именем `mistype*` (наши же uinput) **пропускаются** — иначе
   собственные инжекты Ctrl+C/Ctrl+V «съедаются».

Отладка: `RUST_LOG=debug` → `evdev key …` / `hotkey match` / `hotkey received`.
Проверка: `mistype selftest` (инжект `ctrl+f20` через uinput → assert, что evdev его поймал).

---

## 4. LLM-исправление выделенного текста

**Триггер:** `Alt+R` (по умолчанию) — «LLM-fix selection».

### 4.1 Поток

```
hotkey → handle(LlmFixSelection)
  → cfg.llm.enabled? (нет → Disabled)
  → text_corrector установлен? (нет → уведомление «не настроен llm.base_url»)
  → capture_selection()            // тот же marker-протокол
  → text = captured.trim()
  → пусто? → NoInputFound (без сетевого вызова)
  → notify «LLM: отправляю запрос…»
  → correct_text(text)             // POST {base_url}/v1/chat/completions
  → ошибка → restore clipboard + «LLM не ответил — текст не изменён»
  → пусто / == text → «LLM не нашёл исправлений»
  → replace_with_text(original, corrected, arm_undo=true)
```

### 4.2 HTTP-клиент (`OpenAiCorrector`)

- `POST {base_url}/v1/chat/completions`, Bearer-токен опционален.
- Тело: `messages: [{role:system, content:system_prompt}, {role:user, content:text}]`
  (+ `model`, если задан).
- Ответ парсится как `choices[0].message.content`.
- **Только `spawn_blocking`** — `ureq` блокирующий, синхронный вызов в async
  runtime заморозит демон (нет хоткеев, нет трея).
- `api_token` **никогда не логируется** (doctor печатает `configured/not configured`).

### 4.3 Замена (`replace_with_text`)

Как §1.4 (clipboard → verify → Ctrl+V / type_text → restore), но без layout switch
и с `arm_undo = true`:

1. Запомнить `UndoState { original, replacement, when }`.
2. Положить replacement в буфер (`buffer.set_contents`) — чтобы undo-логика знала длину.

---

## 5. Отмена LLM-замены (Backspace)

**Окно:** 4 секунды после успешной LLM-замены (`UNDO_WINDOW`).

### 5.1 Триггер

Физический `Backspace` (событие `WordEvent::Backspace` из буфера набора).
Событие **не гейтится** `privacy.typing_buffer` — undo должен работать даже
при выключенном буфере.

### 5.2 Алгоритм `on_backspace`

```
suppress? → false (мы сами инжектим)
undo в окне 4s? → нет → обычный Backspace
   ↓ да
physical Backspace уже удалил 1 символ replacement
remaining = len(replacement) - 1
suppress = true                 // игнорировать свои же события
wait_keys_quiet(50, 500)        // дождаться тишины в буфере
backspace(remaining)            // дочистить replacement
layout = detect_lang(original)  // ru или us
type_text(original, layout)     // восстановить оригинал
buffer.set_contents(original)
suppress = false
```

### 5.3 Отмена undo

`on_typing()` — любой другой символ в течение окна → `undo = None`
(пользователь продолжил печатать, откатывать уже поздно).

`clear_undo()` — при toggle-off / quit.

---

## 6. Буфер набора (typing buffer)

**Зачем:** (а) undo LLM-замены, (б) потенциальный fallback для «последнего слова».

- RAM-only, опционален (`privacy.typing_buffer`, `privacy.typing_buffer_max_keys`).
- Никогда не пишется на диск, никогда не логируется.
- При toggle-off — `buffer.clear()` (privacy).
- `key_to_char` мапит **физические** US-коды; живой маппинг идёт через
  `LayoutCharMapper` (текущая раскладка + `en_to_ru_map`).

---

## 7. Конфиг, hotkey, reload

| Событие | Что происходит |
|---------|----------------|
| Settings → «Сохранить» | пишет `config.toml`, создаёт `$TMPDIR/mistype.restart` |
| daemon видит `mistype.restart` | re-exec **с того же PID** (systemd-юнит жив) → hotkey/consume перечитываются |
| Settings → «Перезагрузить конфиг» | пишет `$TMPDIR/mistype.reload` |
| daemon видит `mistype.reload` | перечитывает config + maps + пересобирает `OpenAiCorrector` (~2 c). **Hotkey НЕ перечитываются** — нужен Save/restart |
| Capture в Settings | создаёт `mistype.capture-pause` → daemon игнорирует хоткеи |

Пустой хоткей (`""`) = действие отключено.

---

## 8. Переключение вкл/пауза (toggle)

1. `enabled = !enabled`.
2. Если off: `buffer.clear()`, `clear_undo()`.
3. Notify + обновить иконку (серая = пауза).

---

## 9. Восстановление буфера обмена

| Шаг | Детали |
|-----|--------|
| save | `req_get()` → `SavedClipboard { text: Option<String> }` |
| restore | `req_set(saved.text)` — только если `Some` |
| если restore упал | warning «Не удалось восстановить буфер обмена», конвертация **не** откатывается |
| не-текстовый буфер (картинка) | restore пропускается (документировано) |

**Один инстанс, две trait-вида:** `clipboard: c.clone()`, `selection: c` —
один `ArboardClipboard`, один `last_save`. Два инстанса ломают протокол.

**Почему `wl-copy` предпочтительнее `arboard`:** на Wayland буфер «живёт» в
процессе-владельце. Если процесс умирает / fork'ается из MT-демона — offer
исчезает, `Ctrl+V` вставляет пустоту и **стирает выделение**. `wl-copy`
демонизирует child, offer остаётся. `arboard` держится на отдельном потоке
всю жизнь процесса.

---

## 10. Детект активного окна (focus monitor)

1. `xprop` (X11 / XWayland) — app id.
2. Fallback: KWin D-Bus `getWindowInfo` / `queryWindowInfo` (часто `{}`).
3. `None` → generic GUI-путь, **не блокируем**.

Используется для: `excluded_apps` (keepassxc, …), `disable_in_fullscreen`,
авто-terminal (`terminal_apps`).

---

## 11. Сводка primary/fallback

| Задача | Primary | Fallback |
|--------|---------|----------|
| Хоткей | evdev passive | portal (не реализован) → tray/CLI |
| Выделение | clipboard-marker | — (last word убран) |
| Замена selection | clipboard + Ctrl+V | `type_text` поверх выделения |
| Замена terminal | Ctrl+A/K + Ctrl+Shift+V | backspaces (`terminal_erase`) |
| Layout switch | `org.kde.KeyboardLayouts` | warning (текст не трогаем) |
| Clipboard | `wl-copy` / `wl-paste` | `arboard` (dedicated thread) |
| LLM HTTP | `ureq` в `spawn_blocking` | — |
| Undo LLM | Backspace ≤ 4 s | — |

---

## 12. Ошибки и их последствия

| Ошибка | Реакция |
|--------|---------|
| Нет выделения | `NoInputFound`, тихо |
| EACCES `/dev/input` | лог error, fallback `ManualHotkeySource`, hint `usermod -aG input` |
| EACCES `/dev/uinput` | convert невозможен, tray/settings живы |
| Layout switch fail | **текст не откатываем**, warning |
| Clipboard restore fail | warning, конвертация не откатывается |
| LLM timeout / сеть | restore clipboard, «LLM не ответил — текст не изменён` |
| Panic в адаптере | `catch_unwind` → `Error(recoverable)` → Idle |

---

## 13. Схема состояний

```
Idle
 ├→ Capturing ──→ Converting ──→ Replacing ──→ RestoringClipboard ──→ SwitchingLayout ──→ Idle
 │       │              │              │
 │       └→ NoInputFound              └→ Failed → Error(recoverable) → Idle
 └→ Disabled (toggle / excluded / fullscreen)
```

LLM-путь: `Capturing → (HTTP) → Replacing → Idle`, затем возможен
`Backspace → undo → Idle`.

---

## См. также

- [PLAN.md](PLAN.md) — архитектура, аналоги, roadmap
- [PRIVACY.md](PRIVACY.md), [SECURITY.md](SECURITY.md)
- [MANUAL-TEST-MATRIX.md](MANUAL-TEST-MATRIX.md)
- [AGENTS.md](../AGENTS.md) — landmines, FAQ, как добавить платформу
