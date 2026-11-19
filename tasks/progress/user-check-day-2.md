# День ручної перевірки Fyx, частина 2

Перший день (`user-check-day.md`) більше не проводимо. Його відкриті пункти перенесено сюди. Блок 2
першого дня (UI-розширення), мережу на старті й бренд у файлах координатор уже перевірив керуванням
екраном. Механічні кроки старої чернетки цього дня (рядки демо, реєстр, Inspector) теж перевіряє
координатор, тому їх тут немає.

Тут лишилося те, що може зробити лише людина: **враження** і кроки зі **своїм акаунтом**. Один
прохід на одній збірці `fyx.exe`, приблизно **24 хвилини**. Назви команд, кнопок і повідомлень
наведено англійською, як у Fyx. Ключі, паролі та входи в акаунти вводите лише ви, ніколи не агент.

**Стан аркуша:** 2026-11-19. Місця, які ще треба заповнити, позначені **«ЗАПОВНИТИ: …»**. Шлях і
коміт збірки надсилає гейткіпер. Шукайте `ЗАПОВНИТИ` у файлі: перед перевіркою їх не має лишитися.

| Блок | Що | Хв |
|---|---|---|
| 0 | Підготовка | 3 |
| 1 | Вигляд: іконка, About, панелі | 3 |
| 2 | Діалоги у вікні, Install from File… і Install Dev Extension…, картка дозволу T2 | 5 |
| 3 | Огляд змін агента з Claude Agent (свій акаунт) | 9 |
| 4 | Режим shield (свій ключ Anthropic) | 4 |

Рядки й кнопки звірено з кодом 2026-11-19 на `origin/integration` `0f3bdb411d`.

## 0. Підготовка (3 хв)

### Збірка

- **Коміт:** «ЗАПОВНИТИ: коміт збірки (надсилає гейткіпер)».
- **Файл:** «ЗАПОВНИТИ: шлях до `fyx.exe` (надсилає гейткіпер)».
- `fyx:ui` 0.3 тепер заморожений, тож підходить і release-збірка.
- Перевірка з кореня репозиторію (Git Bash). Кожен рядок має закінчуватися на `yes`. Рядок
  `f868421052` (клавіша Tab в огляді, W5) може бути `NO`: тоді пропустіть крок K1 у блоці 3.

```bash
for c in 37c4a84fe4 a440a97284 dfdb6b0023 3851a95912 f868421052; do git merge-base --is-ancestor $c «ЗАПОВНИТИ: коміт» && echo "$c yes" || echo "$c NO"; done
```

| Коміт | Що | Блок |
|---|---|---|
| `37c4a84fe4` | W3: вибір панелі й підтвердження встановлення у вікні Fyx | 2 |
| `a440a97284` | W3: кнопки **Install from File…** і **Install Dev Extension…** на сторінці Fyx Extensions | 2 |
| `dfdb6b0023` | W3: запит дозволу T2 як картка, а не діалог; затримка кліку 1 с | 2 |
| `3851a95912` | W9: режим shield | 4 |
| `f868421052` | W5: Tab / Shift+Tab / Escape у панелі огляду | 3 |

### Матеріали

Нічого не збирайте самі: усе готує координатор.

- **Демо:** «ЗАПОВНИТИ: теки демо, зібрані з коміту збірки». Теки `demos-0.2\`, `demos-0.2-next\` і
  `demos-0.3\` від 2026-11-18 **не підходять**: ABI 0.2 і 0.3 змінився після `6bc8a1c12b`
  (`accessible-label`, менший 0.3, заморожування). Потрібні `fyx_demo_gallery`, `fyx_demo_images`,
  `fyx_demo_charts` і `fyx_demo_t2`.
- **Пакет:** `D:\fyx-user-check\w3\packages\fyx-demo-counter-0.1.0.fyxext`. «ЗАПОВНИТИ:
  перепакований з коміту збірки, або підтвердити, що старий вантажиться».
- **Проєкт для огляду.** Створіть його заново (той самий скрипт, що в першому дні):

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

### Чистий стан і перший запуск

1. Закрийте **всі** вікна Fyx. Змінні `FYX_UI_DEV_EXTENSIONS` і `FYX_AGENT_REVIEW` мають бути
   порожні: `$env:FYX_UI_DEV_EXTENSIONS; $env:FYX_AGENT_REVIEW`.
2. Новий профіль. Профілі попередніх днів не чіпаємо:

   ```powershell
   Remove-Item -Recurse -Force D:\fyx-user-check\profile-2 -ErrorAction SilentlyContinue
   $fyx = '«ЗАПОВНИТИ: шлях до fyx.exe»'
   $prof = 'D:\fyx-user-check\profile-2'
   & $fyx --user-data-dir $prof --foreground D:\fyx-user-check\review-project
   ```

   Налаштування лежать у `$prof\config\settings.json`, журнал у `$prof\logs\Fyx.log`.
3. Якщо Fyx питає, чи довіряти папці, натисніть **Trust and Continue**.

## 1. Вигляд: іконка, About, панелі (3 хв)

| # | Що зробити | Очікується |
|---|---|---|
| L1 | Подивіться на заголовок вікна, панель задач і Alt+Tab | Скрізь **Fyx** та іконка Fyx, ніде не Zed. Чи гарно виглядає іконка в малому розмірі (панель задач) і у великому (Alt+Tab)? |
| L2 | Палітра команд **Ctrl+Shift+P** → `zed: about` | Вікно **About Fyx**: назва Fyx, рядок `Copyright 2026 Hennadii Shelenhovskyi. Based on Zed, copyright 2022-2025 Zed Industries, Inc.` і примітка `Fyx is an independent fork of Zed and is not affiliated with or endorsed by Zed Industries.` Чи все читається і чи не обрізано? |
| L3 | Пройдіться панелями: дерево проєкту, агентна панель, термінал, **Fyx Extensions** (`fyx_ui: open fyx extensions`) | Ніде немає логотипа чи назви Zed, немає зламаних відступів. Чи зрозуміло з першого погляду, що це за панелі? |

| Крок | Що | Так / ні, враження |
|---|---|---|
| L1 | іконка й назва | |
| L2 | About | |
| L3 | панелі | |

## 2. Діалоги у вікні, встановлення, картка T2 (5 хв)

Діалоги команд UI-розширень тепер малює сам Fyx, усередині вікна і в його стилі, а не системне
вікно Windows:

- **Вибір** — список у стилі палітри команд. Набір тексту фільтрує його, стрілки й Enter обирають,
  Escape або клік поза списком закривають.
- **Підтвердження** — заголовок, опис і кнопки. Enter натискає першу кнопку, Escape скасовує.

| # | Що зробити | Очікується |
|---|---|---|
| W1 | На сторінці **Fyx Extensions** натисніть **Install Dev Extension…**. У вікні вибору теки (заголовок `Install Dev UI Extension`) оберіть теку `fyx_demo_t2` з матеріалів | Підтвердження у вікні Fyx: `Install the dev UI extension fyx-demo-t2?`, під ним `It asks for:` і перелік можливостей простими словами, потім `Component:` і `Folder:`. Кнопки **Install** і **Cancel**. Натисніть **Escape**: нічого не встановлено. Повторіть і натисніть **Install**: toast `Installed the dev UI extension fyx-demo-t2`. |
| W2 | **Install Dev Extension…** ще для `fyx_demo_gallery`, `fyx_demo_images` і `fyx_demo_charts` | Те саме підтвердження для кожної. |
| W3 | **Install from File…**. Вікно вибору файлу (`Install UI Extension Package`) → `fyx-demo-counter-0.1.0.fyxext` | Підтвердження `Install fyx-demo-counter 0.1.0?` з можливостями і шляхом до файлу. **Install** → toast `Installed fyx-demo-counter 0.1.0`, панель Counter у доку. |
| W4 | Палітра команд → `fyx_ui: open extension panel` | Список `Open which extension panel?` з полем `Type to filter…`. Наберіть `cha`: лишається **Charts**. **Enter** відкриває панель. Наберіть щось без збігів: `No match`. |
| W5 | Відкрийте панелі **Gallery 0.2** (або **Gallery**, залежно від збірки демо), **Images** і **Charts**. Лише подивіться | Чи виглядають картинки, діаграма й sparkline охайно, у кольорах теми, без миготіння? |
| W6 | Поставте курсор у `a.txt`. Панель **T2 capabilities** → **Request fetch**. Одразу, менш ніж за секунду, натисніть **Allow**. Потім спробуйте **Enter** і **Escape** | Картка в стосі сповіщень, а не діалог посеред екрана: `Allow T2 capabilities demo to connect to the internet?`. Курсор лишається в `a.txt`. Перша секунда кліки по **Allow** не діють. Enter і Escape картку не закривають і нічого не відповідають. Після секунди **Allow** спрацьовує, на панелі `Permission: allowed`. |

| Крок | Що | Так / ні, враження |
|---|---|---|
| W1–W3 | встановлення через кнопки й підтвердження у вікні | |
| W4 | вибір панелі зі списком і фільтром | |
| W5 | вигляд панелей зображень і діаграм | |
| W6 | картка T2: фокус не забрано, 1 с затримки, клавіші не відповідають | |
| — | чи зручніші діалоги у вікні за системні (одне-два речення) | |

## 3. Огляд змін агента з Claude Agent (9 хв)

Скриптова перевірка W5 уже покрила правила, частковий accept і «changed without review». Тут
лишилося те, чого скрипт не може: справжні клавіші, toast, закриття вкладки і робота зі справжнім
Claude Agent на вашому акаунті.

**Що варто знати:**

- **Огляд ніколи не забирає фокус і не відкриває вкладку сам.** З'являється toast
  `Agent review: N agent edits wait for your review.` з кнопкою **Review**. Ту саму панель
  відкриває **ctrl-k y**.
- **Рішення на весь хід** — **ctrl-k shift-y** / **ctrl-k shift-n**, або кнопки
  **Accept all in turn** / **Reject all in turn**. Коли чекає більше одного файлу, Fyx питає
  `Accept all N waiting files?` (або `Reject all N waiting files?`).
- **Закриття вкладки огляду нічого не вирішує.** Відхилити все — окрема кнопка **Reject and close**.
- **`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.rules`** завжди йдуть на огляд, навіть після «accept all
  in turn». Правило підписане `Fyx's defaults (agent instruction files)`.

**Підготовка:** меню опцій агентної панелі → **Review Agent Edits**; у рядку стану **Review on**.
Новий тред з **Claude Agent**. Перше встановлення завантажує агента (1–2 хв). Увійдіть так, як він
просить, самостійно.

| # | Промпт | Що зробити | Очікується |
|---|---|---|---|
| R1 | *In a.txt change bravo to BRAVO, and in b.txt change delta to DELTA. Make no other change.* | 5 с нічого не натискайте. Подивіться на картку в треді. **Review** у toast. Потім **ctrl-k shift-y** і в запиті **Accept All in Turn** | Фокус і вкладка не змінились самі, є toast з **Review**. Картка в треді: `Fyx is reviewing this edit to … hunk by hunk; decide it in the review pane.` з **Open review**. **Review** відкриває одну панель на хід з обома файлами. Після підтвердження `git -C D:\fyx-user-check\review-project diff` показує обидві зміни. |
| R2 | *In a.txt change charlie to CHARLIE, and in b.txt change echo to ECHO.* | Відкрийте огляд і закрийте вкладку (**ctrl-w**). Потім **ctrl-k y**. Потім **ctrl-k shift-n** → **Reject All in Turn** | Після ctrl-w нічого не записано, знову є toast. ctrl-k y повертає ту саму панель. Після відхилення обидва файли без змін. |
| R3 | *Add the line "Run tests before committing." to AGENTS.md, and in b.txt change foxtrot to FOXTROT.* | **ctrl-k shift-y**, підтвердьте | `b.txt` прийнято, а `AGENTS.md` **лишається** на огляді з підписом `Fyx's defaults (agent instruction files)`. **Reject file** → `git diff AGENTS.md` порожній. |
| K1 | будь-яка правка на огляді | **Tab** кілька разів, **Shift+Tab**, **Escape**. Пропустіть, якщо `f868421052` дав `NO` | Фокус іде кнопками по порядку (**Accept all in turn**, **Reject all in turn**, **Reject and close**, файли, **Accept**, **Reject** …), Shift+Tab назад, Escape повертає в diff. |
| K2 | та сама | **alt-j** / **alt-k**, потім **ctrl-k j** / **ctrl-k k**, потім **alt-c** | Наступний / попередній фрагмент, наступний / попередній файл ходу. alt-c відкриває поле коментаря. |

Наостанок два-три речення про відчуття. Чи не заважає toast? Чи зрозуміло, що робити далі? Чи не
губиться огляд? Чи зручні скорочення **ctrl-k**?

| Крок | Що | Так / ні, враження |
|---|---|---|
| R1 | toast, картка **Open review**, ctrl-k shift-y з підтвердженням | |
| R2 | ctrl-w тримає правки; ctrl-k y; ctrl-k shift-n | |
| R3 | `AGENTS.md` на огляді навіть після accept all | |
| K1–K2 | клавіатура в панелі огляду | |
| — | відчуття (кілька речень) | |

## 4. Режим shield (4 хв)

Потрібен вбудований агент з **вашим** ключем Anthropic. Ключ вводите лише ви: агентна панель →
налаштування провайдерів.

1. Закрийте Fyx. У тому самому PowerShell задайте фальшивий «секрет» і запустіть знову:

   ```powershell
   $env:FYX_CHECK_API_KEY = 'not-a-real-secret'
   & $fyx --user-data-dir $prof --foreground D:\fyx-user-check\review-project
   ```

2. Вимкніть **Review Agent Edits**, щоб огляд не заважав. У `settings.json` додайте
   `"fyx_security": { "mode": "audit" }`.
3. Вбудований агент, новий тред: *Run in the terminal: echo $env:FYX_CHECK_API_KEY. Then create
   .claude/settings.json containing {}.*
   - **Очікується (audit):** у терміналі видно `not-a-real-secret`, файл `.claude\settings.json`
     створено.
   - У журналі (`zed: open log`) є `fyx_guard audit: would blank in an agent terminal:
     FYX_CHECK_API_KEY` і `fyx_guard audit: would block …`.
4. Видаліть `.claude\` і поставте `"mode": "shield"` (діє одразу). Новий тред, той самий промпт.
   - **Очікується:** echo друкує порожній рядок, файл не створено. Агент отримав повідомлення, що
     починається з `Fyx's security shield blocked`.
   - У журналі є `fyx_guard: blocked …`.
5. Поставте `"mode": "off"` і збережіть.
   - **Очікується** вікно з **Lower Protection** і **Keep Current Mode**.
   - **Keep Current Mode** → режим лишається shield.
   - Ще раз збережіть `off` → **Lower Protection**.

| Крок | Що | Так / ні, примітка |
|---|---|---|
| 3 | audit: нічого не блокує, у журналі `would …` | |
| 4 | shield: змінну очищено, запис у `.claude\` заблоковано | |
| 5 | пониження просить підтвердження | |

## Після перевірки

- Надішліть координатору таблиці результатів і враження (або цей файл із заповненими таблицями).
- Знімки екрана помилок кладіть у `D:\fyx-user-check\shots\`.
- Профіль `D:\fyx-user-check\profile-2` можна видалити: дозволи демо й вхід Claude Agent у цьому
  профілі зникнуть разом з ним. Змінну `FYX_CHECK_API_KEY` закрийте разом з PowerShell.
