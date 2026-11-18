# День ручної перевірки Fyx

Один прохід на одній збірці `fyx.exe`, приблизно **45 хвилин** (і ще 6 хвилин на необов'язковий блок 5). Блоки йдуть у такому порядку, щоб
жоден не зіпсував стан для наступного. Назви команд, кнопок і повідомлень наведено англійською, як
у Fyx. Ключі, паролі та входи в акаунти вводите лише ви, ніколи не агент.

| Блок | Що | Хв |
|---|---|---|
| 0 | Підготовка: збірка, профіль, матеріали | 5 |
| 1 | Приватність: нуль мережі на старті; бренд | 7 |
| 2 | UI-розширення (W3): пакети, dev, hot reload, згода, Restart, Inspector | 14 |
| 3 | Огляд змін агента (W5) зі справжнім Claude Agent; моделі при першому використанні | 12 |
| 4 | Режими безпеки (W9): audit і shield | 5 |
| 5 | Необов'язково: демо `fyx:ui` 0.2 (gallery-next, picker, clock, TODO-декорації) | 6 |

Джерела: W3 `tasks/progress/W3-user-check.md` (гілка `claude/fyx-w3-renderer-e20d2b`);
`docs/fyx/agent-review.md`, `docs/fyx/review-rules.md`, `tasks/progress/W5.md`;
`docs/fyx/manual-tests.md` T5; `tasks/progress/W1.md`; `docs/fyx/security-modes.md`.

## 0. Підготовка (5 хв)

### Збірка

Одна збірка на весь день, її підготував координатор:

- **Коміт:** `fyx/main` `6c82da02f3`. Він замінює попередню збірку `493753a224`.
- **Файл:** `D:\fyx-builds\6c82da02f3\fyx.exe`. Він лежить поза всіма target-теками, тож
  прибирання target його не видаляє.
- **Коміти:** усі коміти з таблиці нижче в цій збірці є. Gatekeeper це перевірив, і я перевірив ще
  раз 2026-11-18.
- **Що ще в ній є:**
  - повний список triage-3 для W5 (`6a256f713f`, `4398c314cf`);
  - видимі помилки завантаження W3, виправлення згоди та TOCTOU;
  - декорації W6.

| Що | Коміт | Потрібен для |
|---|---|---|
| W3: install store, `dev_paths`, Restart, згода за рівнями, Inspector | `4ca0073db1` (і попередні на гілці W3) | блок 2 |
| W1: моделі при першому використанні, без локальних probe | `4b83f766d2` | блоки 1 і 3a |
| W1: логотип і копірайт | `41782c1f78`, `4b83f766d2` | блок 1 |
| W5: огляд вбудованого агента, картка **Open review**, правила | `29999cd5f3`, `a98221e345` (уже на `integration`) | блок 3 |
| W9: виправлення shield | `3851a95912` | блок 4 |
| W5: новий UX огляду (toast **Review**, `ctrl-k`), triage-3 | `6a256f713f`, `4398c314cf` | блок 3 |

Повторювати перевірку не обов'язково. Якщо хочеться, виконайте її з кореня репозиторію (Git
Bash); кожен рядок має закінчуватися на `yes`:

```bash
for c in 4ca0073db1 4b83f766d2 41782c1f78 29999cd5f3 a98221e345 3851a95912 6a256f713f 4398c314cf; do git merge-base --is-ancestor $c 6c82da02f3 && echo "$c yes" || echo "$c NO"; done
```

Це debug-збірка. Якщо в ній бракує значків чи шрифтів, скажіть координатору: debug-збірка може
читати ресурси (`assets`) з checkout, у якому її зібрали.

### Чистий стан

1. Закрийте **всі** вікна Fyx: друга копія передає запуск уже відкритій.
2. У PowerShell перевірте, що змінні не задані. Обидві мають бути порожні:

   ```powershell
   $env:FYX_UI_DEV_EXTENSIONS; $env:FYX_AGENT_REVIEW
   ```

3. Весь день працюємо в окремому профілі `D:\fyx-user-check\profile`. Ваш звичайний профіль
   (`%APPDATA%\Fyx`, `%LOCALAPPDATA%\Fyx`) не чіпаємо. Видаліть цей профіль, якщо він лишився з
   минулого разу:

   ```powershell
   Remove-Item -Recurse -Force D:\fyx-user-check\profile -ErrorAction SilentlyContinue
   ```

4. Змінні для зручності (у тому самому вікні PowerShell, яке буде відкрите весь день):

   ```powershell
   $fyx = 'D:\fyx-builds\6c82da02f3\fyx.exe'
   $prof = 'D:\fyx-user-check\profile'
   ```

   У цьому профілі налаштування лежать у `$prof\config\settings.json`, дані в `$prof`, журнал у
   `$prof\logs\Fyx.log`.

### Матеріали

- `D:\fyx-user-check\w3\`: `packages\fyx-demo-counter-0.1.0.fyxext`, `…-0.2.0.fyxext`,
  `dev-todo\`, `crash-item\` (вже є; як створити заново, описано в W3-user-check.md, «Підготовка»).
- Проєкт для огляду змін, створіть його зараз:

  ```powershell
  $p = 'D:\fyx-user-check\review-project'
  Remove-Item -Recurse -Force $p -ErrorAction SilentlyContinue
  New-Item -ItemType Directory "$p\.fyx" | Out-Null
  $lines = 'alpha','bravo','charlie','delta','echo','foxtrot','golf'
  Set-Content "$p\a.txt" $lines; Set-Content "$p\b.txt" $lines
  Set-Content "$p\.env" 'FAKE_SECRET=not-a-real-secret'
  Set-Content "$p\.fyx\review.toml" 'deny = ["*.env"]'
  Set-Content "$p\AGENTS.md" '# Notes for agents'
  git -C $p init -q; git -C $p add -A; git -C $p commit -qm start
  ```

## 1. Приватність і бренд (7 хв)

Цей блок має бути **першим**: профіль ще порожній, ключів немає.

### 1a. Нуль мережі на старті

1. Відкрийте **Resource Monitor** (`resmon` у меню Пуск) → вкладка **Network**.
2. Запустіть Fyx без папки:

   ```powershell
   & $fyx --user-data-dir $prof --foreground
   ```

3. У Resource Monitor поставте галочку біля `fyx.exe` у **Processes with Network Activity**. Протягом
   60 секунд дивіться **Network Activity** і **TCP Connections**.
4. **Очікується**: жодної адреси, крім `127.0.0.1` і `::1`. Зі збіркою, що містить `4b83f766d2`, не
   має бути й локальних звернень на порти `1234` і `8080` (LM Studio, llama.cpp). Короткі
   з'єднання Resource Monitor може пропустити. Для точнішої перевірки є скрипт з
   `docs/fyx/manual-tests.md`, T5, крок 2.

### 1b. Бренд

1. **Очікується**: у вікні, заголовку й на панелі задач **Fyx**, іконка Fyx, а не Zed. На сторінці
   привітання логотип Fyx.
2. Відкрийте **About** (палітра команд **Ctrl+Shift+P** → `zed: about`). **Очікується**: назва Fyx,
   рядок `Copyright 2026 Hennadii Shelenhovskyi. Based on Zed, copyright 2022-2025 Zed Industries,
   Inc.` і примітка `Fyx is an independent fork of Zed and is not affiliated with or endorsed by
   Zed Industries.` Під час відкриття About у Resource Monitor не з'являється нових адрес.
3. Властивості файлу `fyx.exe` (правий клік → **Properties** → **Details**): **Product name**
   `Fyx Dev`, **Copyright** з тим самим рядком.
4. Сторінка ліцензії в інсталяторі: **пропускаємо**, інсталятор не збирався (немає Inno Setup).
   Якщо координатор дасть інсталятор, на сторінці ліцензії має бути GPL, а не Terms of Service
   Zed.

| Крок | Що | Так / ні, примітка |
|---|---|---|
| 1a | 60 с без адрес, крім loopback | |
| 1a | без звернень на `:1234` і `:8080` | |
| 1b | назва, іконка, логотип | |
| 1b | About: копірайт і примітка, без мережі | |
| 1b | властивості `fyx.exe` | |
| 1b | сторінка ліцензії в інсталяторі (якщо зібраний) | |

## 2. UI-розширення (14 хв)

Повний опис кожного кроку є в `tasks/progress/W3-user-check.md`. Нижче той самий порядок,
скорочено; усе робиться в тому самому вікні Fyx. Команди запускаються з палітри
(**Ctrl+Shift+P**). Відкрийте папку `D:\fyx-user-check\review-project` (**File → Open Folder**).

1. **Пакет.** `fyx_ui: install UI extension from file` → `…\packages\fyx-demo-counter-0.1.0.fyxext`.
   - **Очікується** діалог **Install fyx-demo-counter 0.1.0?** з `ui:panel`, позначка
     `Unsigned, not from the registry`.
   - **Install** → панель Counter у доку, **без** запиту **Enable**. Натисніть **Increment** двічі.
2. **Оновлення.** Те саме з `…-0.2.0.fyxext`.
   - **Очікується** `Installed now: 0.1.0`, потім
     `Installed fyx-demo-counter 0.2.0 (previous: 0.1.0)`.
   - Панель на місці, лічильник 0.
3. **Відкат.** `fyx_ui: roll back UI extension` → **fyx-demo-counter**.
   - **Очікується** `Rolled fyx-demo-counter back to 0.1.0`.
4. **Pin.** `fyx_ui: toggle pin UI extension` → `Pinned fyx-demo-counter to 0.1.0`.
   - Спроба встановити 0.2.0 → `fyx-demo-counter is pinned to 0.1.0; unpin it to install 0.2.0`.
   - Ще раз toggle → `Unpinned fyx-demo-counter`.
5. **Dev-папка.** `fyx_ui: install dev UI extension` → `D:\fyx-user-check\w3\dev-todo`.
   - **Install** → панель Todo.
   - У `$prof\config\settings.json` з'явився `"dev_paths"` з цією папкою.
6. **Hot reload.** Додайте пункт у Todo, потім виконайте в PowerShell:

   ```powershell
   (Get-Item D:\fyx-user-check\w3\dev-todo\fyx_ui.wasm).LastWriteTime = Get-Date
   ```

   - **Очікується**: приблизно за 1 с панель перемонтувалась на тому самому місці, ваш пункт зник.
7. **Невдале перезавантаження.**

   ```powershell
   Copy-Item D:\fyx-user-check\w3\dev-todo\fyx_ui.wasm D:\fyx-user-check\w3\fyx_ui.wasm.bak
   Set-Content D:\fyx-user-check\w3\dev-todo\fyx_ui.wasm "broken"
   ```

   - **Очікується**: Todo працює далі, унизу `Reload failed: …` і **Retry**.
   - Поверніть файл:

     ```powershell
     Copy-Item D:\fyx-user-check\w3\fyx_ui.wasm.bak D:\fyx-user-check\w3\dev-todo\fyx_ui.wasm -Force
     ```

   - **Очікується**: рядок помилки зник сам.
8. **Згода.** У `settings.json` допишіть до `dev_paths` другу папку,
   `"D:\\fyx-user-check\\w3\\crash-item"`, і збережіть.
   - **Очікується** сповіщення
     `The UI extension fyx-test-status-item is installed but not enabled. It asks for: …` з
     **Enable** і **Not now**.
   - **Enable** → у рядку стану `Pings: 0`, **Ping**, **Crash**.
9. **Restart extension.** **Ping** → `Pings: 1`. **Crash** →
   `fyx-test-status-item stopped: …` і **Restart extension**.
   - Натисніть **Restart extension** → знову `Pings: 0`.
10. **Inspector.** У `settings.json` додайте `"developer_mode": true` у
    `"fyx": { "extensions": { … } }`, збережіть і відкрийте нове вікно (**Ctrl+Shift+N**).
    - `fyx_ui: toggle extension inspector` → кнопка `fyx-demo-counter/counter` → у Counter
      натисніть **Increment**.
    - **Очікується**: у **Tree** рядок `"Count: 1"`, в **Events and updates** `event Click(5)` і
      `set-props`, у **Calls** час набагато менший за `200 ms`.
    - Закрийте друге вікно.

Прибирання й перевірку після перезапуску (кроки 5 і 12 у W3) пропускаємо. Розширення лишаються і
не заважають блоку 3, а після перезапуску в блоці 3a видно, що Counter на місці без запитів.

| Крок | Що | Так / ні, примітка |
|---|---|---|
| 1 | пакет встановлено, без Enable | |
| 2 | оновлення до 0.2.0 | |
| 3 | відкат до 0.1.0 | |
| 4 | pin відмовляє 0.2.0; unpin | |
| 5 | dev-встановлення, запис у `dev_paths` | |
| 6 | hot reload ~1 с, панель на місці | |
| 7 | `Reload failed` зі старою версією; самовідновлення | |
| 8 | Enable для доданої вручну папки | |
| 9 | Crash → **Restart extension** | |
| 10 | Inspector: дерево, події, час | |

## 3. Огляд змін агента (12 хв)

Скриптова перевірка W5 на `6c82da02f3` уже пройшла (`tasks/progress/W5.md`). Цей блок перевіряє
те, що скрипт не може: справжні клавіші, toast, закриття вкладки та роботу зі справжнім Claude
Agent.

### 3a. Ключ і моделі при першому використанні (3 хв)

1. В агентній панелі (**Ctrl+?** або кнопка агента в доку) відкрийте налаштування провайдерів і
   введіть **свій** ключ Anthropic. Ключ вводите лише ви.
2. Закрийте Fyx і запустіть знову, вже з папкою:

   ```powershell
   & $fyx --user-data-dir $prof --foreground D:\fyx-user-check\review-project
   ```

3. **Очікується**: панель Counter на місці без діалогів (перевірка після перезапуску з блоку 2).
   Протягом 30 с у Resource Monitor для `fyx.exe` немає адрес, крім loopback, хоча ключ є і папка
   відкрита.
4. Відкрийте вибір моделі в агентній панелі. **Очікується**: протягом кількох секунд з'являється
   з'єднання з адресою поза машиною (Anthropic), у списку є моделі.

### 3b. Огляд

Що нового в UX огляду (збірка `6c82da02f3`):

- **Огляд ніколи не забирає фокус і не відкриває вкладку сам.** З'являється toast
  `Agent review: … wait for your review.` з кнопкою **Review**. Ту саму панель відкриває
  `ctrl-k y` (`agent_review: open reviews`).
- **Рішення на весь хід** — `ctrl-k shift-y` / `ctrl-k shift-n`, або кнопки
  **Accept all in turn** / **Reject all in turn**. Вони діють лише в панелі огляду (контекст
  `AgentReview`). Коли чекає більше одного файлу, Fyx питає `Accept all N waiting files?` з
  кнопками **Accept All in Turn** і **Cancel**. **Stop accepting** скасовує рішення на решту
  ходу.
- **Закриття вкладки огляду нічого не вирішує.** Правки лишаються затриманими, і знову
  з'являється toast. Відхилити все — це окрема кнопка **Reject and close**.
- **Файли інструкцій для агентів** (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.rules`) завжди
  йдуть на огляд (`always_ask`), навіть після «accept all in turn». Правило в панелі підписане
  `Fyx's defaults (agent instruction files)`.

Кроки:

1. Увімкніть огляд: меню опцій агентної панелі → **Review Agent Edits**. **Очікується** у рядку
   стану **Review on**.
2. Створіть новий тред з **Claude Agent**. Перше встановлення завантажує агента, це 1–2 хв. Увійдіть
   так, як він просить, самостійно.

Скриптова перевірка W5 на `6c82da02f3` уже пройшла і покрила правила (`deny`, `auto_accept`),
частковий accept, список «changed without review» і delete/move вбудованого агента. Тут лишилося
те, чого скрипт не може: справжні клавіші в контексті `AgentReview`, закриття вкладки, toast і
`ctrl-k y`, і відчуття від роботи зі справжнім Claude Agent.

| # | Промпт | Що зробити | Очікується |
|---|---|---|---|
| R1 | *In a.txt change bravo to BRAVO, and in b.txt change delta to DELTA. Make no other change.* | 5 с нічого не натискайте. Подивіться на картку запиту в треді. Натисніть **Review** у toast. Потім **ctrl-k shift-y** і в запиті **Accept All in Turn** | Фокус і активна вкладка самі не змінились, з'явився toast з **Review**. Картка в треді: `Fyx is reviewing this edit to … hunk by hunk; decide it in the review pane.` і **Open review** замість Allow / Reject. У рядку стану `Review: N waiting`. **Review** відкриває одну панель на хід з обома файлами. ctrl-k shift-y питає `Accept all 2 waiting files?`; після підтвердження `git -C D:\fyx-user-check\review-project diff` показує обидві зміни. |
| R2 | *In a.txt change charlie to CHARLIE, and in b.txt change echo to ECHO.* | Відкрийте огляд, закрийте його вкладку (**ctrl-w**). Потім **ctrl-k y**. Потім **ctrl-k shift-n**, у запиті спершу **Cancel**, тоді ще раз **ctrl-k shift-n** і **Reject All in Turn** | Після закриття вкладки нічого не записано (`git diff` показує лише зміни R1), і знову є toast. ctrl-k y повертає ту саму панель. Перше ctrl-k shift-n питає `Reject all 2 waiting files?`; після **Cancel** нічого не вирішено. Після **Reject All in Turn** обидва файли без змін. |
| R3 | *Add the line "Run tests before committing." to AGENTS.md, and in b.txt change foxtrot to FOXTROT.* | **ctrl-k shift-y**, підтвердьте | `b.txt` прийнято. `AGENTS.md` **лишається** на огляді, з підписом `Fyx's defaults (agent instruction files)`. Відхиліть його (**Reject file**) і перевірте, що `git diff AGENTS.md` порожній. |

Наостанок запишіть два-три речення про відчуття: чи не заважає toast, чи зрозуміло, що робити далі,
чи не губиться огляд.

**Необов'язково** (скрипт W5 це вже перевірив; робіть, лише якщо лишився час):

| # | Промпт | Очікується |
|---|---|---|
| O1 | *In b.txt change golf to GOLF, and add the line TOKEN=abc to .env.* | `.env` не змінено; у панелі `by rule deny = "*.env" in …\.fyx\review.toml`. |
| O2 | *In a.txt change golf to GOLF, then run in the terminal: echo hello > c.txt* | У панелі ходу `1 file changed without review` з `c.txt`. |
| O3 | Вбудований агент: *delete c.txt and rename a.txt to a2.txt* | Звичайний дозвіл, як у Zed, без панелі огляду. |

Журнал (`zed: open log`): немає `panic`, є рядки `fyx: agent review:`.

| Крок | Що | Так / ні, примітка |
|---|---|---|
| 3a | Counter на місці після перезапуску | |
| 3a | з ключем і папкою: 30 с без мережі | |
| 3a | мережа лише після відкриття вибору моделі | |
| R1 | фокус не забрано; toast **Review**; картка **Open review**; `ctrl-k shift-y` питає підтвердження | |
| R2 | ctrl-w тримає правки; `ctrl-k y`; `ctrl-k shift-n` з Cancel і з підтвердженням | |
| R3 | `AGENTS.md` на огляді навіть після accept all | |
| — | відчуття від роботи зі справжнім Claude Agent (кілька речень) | |
| O1–O3 | необов'язкові кроки (якщо робили) | |
| — | журнал без panic | |

## 4. Режими безпеки (5 хв)

Збірка містить виправлення W9 (`3851a95912`), тож блок обов'язковий.

1. Закрийте Fyx. У тому самому PowerShell задайте фальшивий «секрет» і запустіть знову:

   ```powershell
   $env:FYX_CHECK_API_KEY = 'not-a-real-secret'
   & $fyx --user-data-dir $prof --foreground D:\fyx-user-check\review-project
   ```

2. Вимкніть огляд (**Review Agent Edits**), щоб не заважав. У `settings.json` додайте
   `"fyx_security": { "mode": "audit" }`.
3. Вбудований агент, новий тред: *Run in the terminal: echo $env:FYX_CHECK_API_KEY. Then create
   .claude/settings.json containing {}.*
   - **Очікується** (audit): у терміналі видно `not-a-real-secret`, файл `.claude\settings.json`
     створено.
   - У журналі є `fyx_guard audit: would blank in an agent terminal: FYX_CHECK_API_KEY` і
     `fyx_guard audit: would block an agent write to …`.
4. Видаліть `.claude\`, поставте `"mode": "shield"` (підвищення діє одразу). Новий тред, той самий
   промпт.
   - **Очікується**: echo друкує порожній рядок, файл не створено, агент отримав повідомлення з
     `read-only`.
   - У журналі `fyx_guard: blocked …`.
5. Поставте `"mode": "off"` і збережіть.
   - **Очікується** вікно з **Lower Protection** і **Keep Current Mode**.
   - **Keep Current Mode** → режим лишається shield.
   - Ще раз збережіть `off` → **Lower Protection**.

| Крок | Що | Так / ні, примітка |
|---|---|---|
| 3 | audit: нічого не блокує, журнал `would …` | |
| 4 | shield: змінну очищено, запис у `.claude\` заблоковано | |
| 5 | пониження просить підтвердження | |

## 5. Необов'язково: демо `fyx:ui` 0.2 (6 хв)

Цей блок робіть, лише якщо лишився час. Він показує, що додає 0.2: модальне вікно з вибором з
клавіатури, годинник у рядку стану на тіках і декорації в редакторі.

**Підготовка.**

1. Компоненти 0.2 вже готові. W4 зібрав їх із того самого `fyx/main` `6c82da02f3` у теки
   `D:\fyx-user-check\demos-0.2\fyx_demo_gallery\` і `…\fyx_demo_decorations\`.
   - У кожній теці є `extension.toml` з `api_version = "0.2.0"` і `fyx_ui.wasm`. Я перевірив це
     2026-11-18.
   - Gallery просить `ui:panel`, `ui:status-bar` і `ui:modal`, а декорації — `ui:panel` і
     `ui:editor-decorations`.
2. Проєкт, у якому декорації мають до чого прив'язатися (ті самі шляхи, що в демо):

   ```powershell
   $t = 'D:\fyx-user-check\todo-project'
   New-Item -ItemType Directory -Force "$t\src" | Out-Null
   Set-Content "$t\src\main.rs" 'fn main() {', '    // TODO: read the config path from the command line', '    let config = load_config("fyx.toml");', '    // FIXME(sam): this panics when the file is missing', '    run(config.unwrap());', '}'
   ```

3. Допишіть обидві теки до `"dev_paths"` у `$prof\config\settings.json` (два записи з блоку 2
   лишаються) і збережіть:

   ```json
   "fyx": { "extensions": { "dev_paths": [
     "D:\\fyx-user-check\\w3\\dev-todo",
     "D:\\fyx-user-check\\w3\\crash-item",
     "D:\\fyx-user-check\\demos-0.2\\fyx_demo_gallery",
     "D:\\fyx-user-check\\demos-0.2\\fyx_demo_decorations"
   ] } }
   ```

   Потім відкрийте `D:\fyx-user-check\todo-project`. Для кожної нової теки з'явиться **Enable**,
   натисніть його.

| # | Що зробити | Очікується |
|---|---|---|
| D1 | Відкрийте панель **Gallery 0.2** (значок `bolt`, правий док). Наведіть курсор на кнопки, наберіть щось у прихованому полі, натисніть **Refresh rows** | Кнопки мають підказки (tooltip). Поле приховує текст і показує `Token: N characters`. Рядки дерева мають відступи за глибиною (indent) і після Refresh читаються `(scan N)`. |
| D2 | **Pick a fruit**, наберіть кілька літер, стрілки вниз і вгору, **Enter**. Потім ще раз **Pick a fruit** і **Escape** | Відкривається модальне вікно **Pick a fruit**, фокус у полі фільтра. Стрілки рухають виділення. Enter закриває вікно, а на панелі `Picked: <fruit>`. Після Escape на панелі `Picked: nothing (escape)`. |
| D3 | Подивіться на правий край рядка стану | Значок годинника і час UTC, щосекунди новий. |
| D4 | Відкрийте панель **TODO highlights** (лівий док), **Highlight all**, відкрийте `src/main.rs` | Над рядками з `TODO` і `FIXME` з'являються блоки декорацій, зі значком у кольорі свого виду. Додайте рядок над TODO: блок їде разом зі своїм рядком. **Done** у блоці прибирає його, і панель оновлюється. |

| Крок | Що | Так / ні, примітка |
|---|---|---|
| D1 | панель 0.2: tooltip, indent, приховане поле | |
| D2 | модальне вікно з клавіатури, `Picked: …` | |
| D3 | годинник на тіках | |
| D4 | декорації над TODO/FIXME, ідуть за рядком, **Done** | |

## Після перевірки

- Надішліть координатору таблиці результатів (або цей файл із заповненими таблицями).
- Знімки екрана помилок кладіть у `D:\fyx-user-check\shots\`.
- Профіль `D:\fyx-user-check\profile` можна видалити. Ключ Anthropic лишається в Windows Credential
  Manager. Якщо він там більше не потрібен, видаліть його там самі.
