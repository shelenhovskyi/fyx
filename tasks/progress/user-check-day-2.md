# День ручної перевірки Fyx, частина 2

Другий прохід, після першого дня (`user-check-day.md`). Він перевіряє те, що злилося після
`6c82da02f3`:

- демо `fyx:ui` 0.2 у самому застосунку, зокрема сповіщення й прогрес;
- 0.3: зображення, canvas і дозволи T2;
- локальний реєстр;
- розділ Inspector для мережі, програм і правок;
- клавіатурну навігацію в огляді змін агента.

Один прохід на одній збірці `fyx.exe`, приблизно **45 хвилин**. Назви команд, кнопок і повідомлень
наведено англійською, як у Fyx. Ключі, паролі та входи в акаунти вводите лише ви, ніколи не агент.

**Стан аркуша:** чернетка від 2026-11-18. Місця, які ще треба заповнити, позначені
**«ЗАПОВНИТИ: …»**. Їх заповнює координатор, коли буде збірка й матеріали. Шукайте `ЗАПОВНИТИ` у
файлі: перед перевіркою їх не має лишитися.

| Блок | Що | Хв |
|---|---|---|
| 0 | Підготовка: збірка, профіль, матеріали | 5 |
| 1 | Демо 0.2: gallery-next, picker, годинник, TODO-декорації, сповіщення й прогрес | 9 |
| 2 | 0.3: зображення і canvas | 5 |
| 3 | 0.3: дозволи T2 з `fyx_demo_t2` | 12 |
| 4 | Реєстр: встановлення, оновлення, затримане оновлення T2, блокування, відкат | 9 |
| 5 | Inspector: мережа, програми, правки | 3 |
| 6 | Огляд змін агента: Tab і клавіатура | 3 |

Джерела:
- W4: `tasks/progress/W4.md` (демо);
- W3: `tasks/progress/W3.md` (згода T2, сторінка Fyx Extensions, реєстр, Inspector);
- W2c: `tasks/progress/W2c.md` (хост 0.3);
- W5: `tasks/progress/W5.md` (клавіші огляду);
- дизайн: `docs/fyx/wit-0.2/design.md` §M і `docs/fyx/wit-0.3/design.md` §7–§13.

Рядки й кнопки звірено з кодом 2026-11-18: `origin/integration` `ce413d1733` і верхівки гілок W2c
(`107e36ab95`), W3 (`9df6ddb532`), W4 (`9cd5a28180`), W5 (`f868421052`).

## 0. Підготовка (5 хв)

### Збірка

Потрібна **debug-збірка**: світ 0.3 поки staged, і його вантажать лише dev-збірки.

- **Коміт:** «ЗАПОВНИТИ: коміт `fyx/main`».
- **Файл:** «ЗАПОВНИТИ: шлях, наприклад `D:\fyx-builds\<коміт>\fyx.exe`».
- **Коміти:** у збірці мають бути всі коміти з таблиці нижче. Без них відповідні кроки не
  спрацюють:

| Що | Коміт | Потрібен для |
|---|---|---|
| W2c: інтерфейс `permissions` 0.3 у хості (стан і запит дозволу) | `4ecf7ef885` | блок 3 |
| W4: кнопки **Request …** у `fyx_demo_t2`, стан дозволу при відкритті | `9cd5a28180` | блок 3 |
| W3: хост `workspace:edit` і `agent:observe` | `f5a3b863fc` | блок 3, кроки T3, T4, T9 |
| W3: розділ Inspector «Network, programs, edits» | `885f563c50` | блок 5 |
| W5: Tab / Shift+Tab / Escape у панелі огляду | `f868421052` | блок 6 |
| W4: **Notify** і **Run job** у gallery-next, `ui:notifications` | `27e6220906` | блок 1, кроки N1–N6 |
| `integration`, з якого W4 зібрав демо (виправлення `fyx_demo_t2` для T6 і недовіреної папки) | `6bc8a1c12b` | блоки 1–3 |
| W3: правильні описи `ui:editor-decorations` і `ui:notifications` у картці згоди | «ЗАПОВНИТИ: коміт W3, ще не зроблено» | блок 1 |

Перевірка з кореня репозиторію (Git Bash). Кожен рядок має закінчуватися на `yes`:

```bash
for c in 4ecf7ef885 9cd5a28180 f5a3b863fc 885f563c50 f868421052 27e6220906 6bc8a1c12b; do git merge-base --is-ancestor $c «ЗАПОВНИТИ: коміт» && echo "$c yes" || echo "$c NO"; done
```

### Чистий стан

1. Закрийте **всі** вікна Fyx.
2. Змінні `FYX_UI_DEV_EXTENSIONS` і `FYX_AGENT_REVIEW` мають бути порожні:

   ```powershell
   $env:FYX_UI_DEV_EXTENSIONS; $env:FYX_AGENT_REVIEW
   ```

3. Окремий профіль для цього дня, `D:\fyx-user-check\profile-2`. Профіль першого дня не чіпаємо.

   ```powershell
   Remove-Item -Recurse -Force D:\fyx-user-check\profile-2 -ErrorAction SilentlyContinue
   $fyx = '«ЗАПОВНИТИ: шлях до fyx.exe»'
   $prof = 'D:\fyx-user-check\profile-2'
   ```

   Налаштування лежать у `$prof\config\settings.json`, журнал у `$prof\logs\Fyx.log`.

4. Потрібні мережа (блок 3 звертається до `api.github.com`) і `git` у `PATH`. Перевірка:
   `git --version`.

### Матеріали

Нічого не збирайте самі: усе готує координатор.

Демо вже готові. W4 зібрав їх з `integration` `6bc8a1c12b` (debug, `wasm32-wasip2`), і кожне
проходить `fyx-ext check`. Я перевірив теки й маніфести 2026-11-18.

- **Демо 0.2** у `D:\fyx-user-check\demos-0.2\`:
  - `fyx_demo_gallery\`: перезібрана, з **Notify** і **Run job**, просить `ui:panel`,
    `ui:status-bar`, `ui:modal` і `ui:notifications`. Вона замінила копію першого дня.
  - `fyx_demo_decorations\`: та сама, що в перший день, просить `ui:panel` і
    `ui:editor-decorations`.
- **Демо 0.3** у `D:\fyx-user-check\demos-0.3\`, кожне з `api_version = "0.3.0"`:
  - `fyx_demo_images\`: з `assets\` (`logo.svg`, `sample.png`, `kinds\`), просить `ui:panel` і
    `ui:images`;
  - `fyx_demo_charts\`: просить `ui:panel` і `ui:canvas`;
  - `fyx_demo_t2\`: просить `ui:panel`, `network:fetch`, `process:spawn`, `workspace:edit` і
    `agent:observe`.
- **Реєстри:** три теки, які готує `script/fyx-test-registry`. Опис у блоці 4.
  - `D:\fyx-user-check\registry-1\`: `fyx-demo-counter` 0.1.0.
  - `D:\fyx-user-check\registry-2\`: `fyx-demo-counter` 0.2.0, а також оновлення, що додає домен
    або програму (для затриманого оновлення T2).
  - `D:\fyx-user-check\registry-3\`: те саме, плюс блок-лист, що блокує `fyx-demo-counter` 0.2.0.
  - «ЗАПОВНИТИ: скрипт зараз робить одну версію на демо, порожній блок-лист і жодного релізу T2;
    див. "Відомі прогалини" в кінці».
- **Проєкт для T2 і декорацій.** Створіть його зараз:

  ```powershell
  $t = 'D:\fyx-user-check\t2-project'
  Remove-Item -Recurse -Force $t -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Force "$t\src" | Out-Null
  Set-Content "$t\README.md" '# T2 test project'
  Set-Content "$t\src\main.rs" 'fn main() {', '    // TODO: read the config path from the command line', '    let config = load_config("fyx.toml");', '    // FIXME(sam): this panics when the file is missing', '    run(config.unwrap());', '}'
  git -C $t init -q; git -C $t add -A; git -C $t commit -qm start
  ```

- **Проєкт для огляду змін** (блок 6): той самий скрипт `review-project`, що в першому дні
  (`user-check-day.md`, блок 0, «Матеріали»). Запустіть його ще раз, щоб проєкт був чистим.

### Перший запуск

1. Допишіть теки демо до налаштувань. Створіть `$prof\config\settings.json` з таким вмістом:

   ```json
   {
     "fyx": { "extensions": {
       "developer_mode": true,
       "dev_paths": [
         "D:\\fyx-user-check\\demos-0.2\\fyx_demo_gallery",
         "D:\\fyx-user-check\\demos-0.2\\fyx_demo_decorations",
         "D:\\fyx-user-check\\demos-0.3\\fyx_demo_images",
         "D:\\fyx-user-check\\demos-0.3\\fyx_demo_charts",
         "D:\\fyx-user-check\\demos-0.3\\fyx_demo_t2"
       ]
     } }
   }
   ```

2. Запустіть Fyx і відкрийте проєкт:

   ```powershell
   & $fyx --user-data-dir $prof --foreground D:\fyx-user-check\t2-project
   ```

3. Якщо Fyx питає, чи довіряти папці, натисніть **Trust and Continue**. У заголовку не має бути
   **Restricted Mode**. Без довіри демо в блоці 3 пише `Trust this project to use git` або
   `Trust this project to edit its files` замість результату.
4. Для кожної теки з'явиться картка
   `The UI extension <id> is installed but not enabled. It asks for: …` з **Enable** і **Not now**.
   Натисніть **Enable** для всіх п'яти.
   - **Очікується:** у картці кожна можливість описана по суті. Для `ui:editor-decorations` і
     `ui:notifications` **не** має бути `Not used by this version of Fyx (…)`. Якщо такий текст
     є, запишіть: це ще не злите виправлення W3.

| Крок | Що | Так / ні, примітка |
|---|---|---|
| 0 | усі коміти в збірці (`yes`) | |
| 0 | п'ять карток **Enable**; описи можливостей без `Not used by this version of Fyx` | |

## 1. Демо 0.2 у застосунку (9 хв)

Перший день уже перевірив gallery-next, picker, годинник і декорації в необов'язковому блоці 5.
Тут вони коротко, а нове — сповіщення й прогрес.

| # | Що зробити | Очікується |
|---|---|---|
| D1 | Відкрийте панель **Gallery 0.2** (значок `bolt`, правий док) | Заголовок `fyx:ui 0.2`. Кнопки **Copy**, **Pin**, **Delete** мають підказки; після кліку `Last action: Copy` тощо. Поле `Paste a token` приховує текст і показує `Token: N characters`. |
| D2 | **Pick a fruit**, наберіть `an`, стрілки, **Enter**. Ще раз **Pick a fruit** і **Escape** | Модальне вікно **Pick a fruit**, фокус у полі `Filter fruits`, підказка `Up and Down select, Enter picks, Escape closes`. Після Enter на панелі `Picked: <fruit>`, після Escape `Picked: nothing (escape)`. |
| D3 | Правий край рядка стану | Годинник `HH:MM:SS UTC`, щосекунди новий. |
| D4 | Панель **TODO highlights** (лівий док) → **Highlight all**, відкрийте `src/main.rs` | Підсумок `6 comments in 4 files, …`. Над рядками з `TODO` і `FIXME` блоки з **Done**. Блок їде разом зі своїм рядком; **Done** прибирає його. Блоки є лише для `src/main.rs`, бо інших файлів демо в проєкті немає. |

**Сповіщення й прогрес.** На панелі **Gallery 0.2**, розділ `Notifications and progress`: кнопки
**Notify** і **Run job**. Під ними рядки `Notifications: none yet` і `Job: not started`. Поставте
курсор у `src/main.rs` і клацайте кнопки мишею, щоб було видно, що фокус лишається в редакторі.

| # | Що зробити | Очікується |
|---|---|---|
| N1 | **Notify**. Одразу (менш ніж за секунду) спробуйте натиснути **Open** | З'являються дві картки, фокус лишається в редакторі. Перша — інформаційна, `Gallery says hello`, текст `A demo notification from the gallery's 0.2 build.`, кнопки **Open** і **Later**. Сама вона не зникає. Друга — попередження `This one closes by itself`. Угорі кожної рядок `Gallery demo · fyx-demo-gallery`. Перші 750 мс **Open** неактивна, і швидкий клік нічого не робить. На панелі `Notifications: shown #<a> and #<b>`. |
| N2 | Зачекайте 10 с, не наводячи курсор на попередження | Попередження зникає саме, на панелі `Notifications: #<b> closed by itself`. Інформаційна картка лишається. |
| N3 | На інформаційній картці **Open**. Потім ще раз **Notify** і закрийте нову інформаційну картку хрестиком (**Close**) | Після **Open** картка закривається, на панелі `Notifications: #<a> pressed open`. Після хрестика `Notifications: #<c> closed by you`. |
| N4 | **Run job**. Наведіть курсор на значок у лівій частині рядка стану, потім клацніть його | У лівій частині рядка стану значок `N%`, `Gallery demo` і `Gallery job`. Під заголовком панелі смужка `Gallery job`. Обидва ростуть на 10 % що пів секунди, а на панелі `Job: 10 %`, `Job: 20 %` … Підказка: `Gallery demo · fyx-demo-gallery` і `Gallery job (N%): Step k of 10`. Через 5 с на панелі `Job: done`, і обидва зникають. |
| N5 | Ще раз **Run job**, на 30–40 % клацніть значок у рядку стану → `Cancel Gallery job` | Меню з пунктом `Cancel Gallery job` (і `Show …`). Після Cancel обидва прогреси зникають, на панелі `Job: cancelled`. |
| N6 | Необов'язково: **Notify** тричі поспіль | Відкритих карток від gallery не більше 3. Зайві не показані, а в журналі (`zed: open log`) є рядок `notification not shown (…)`. |

**Що показує Fyx для сповіщень і прогресу** (код W3 на `integration`; потрібен, щоб зрозуміти
кроки N1–N6):

- **Сповіщення** — картка у стосі сповіщень вікна. **Фокус вона не забирає**: курсор лишається там,
  де був.
  - Угорі рядок походження: значок, назва розширення (`Gallery demo`) і `· fyx-demo-gallery`.
  - Далі заголовок зі значком рівня, текст і до двох кнопок дій.
  - Кнопки дій перші **750 мс** неактивні, щоб випадковий клік не спрацював. Хрестик **Close**
    і **Escape** закривають картку одразу.
  - Інформаційна картка зникає сама за 6 с, попередження й помилки за 10 с. Поки курсор над
    карткою, таймер стоїть.
  - Одночасно відкритих карток від одного розширення не більше 3, і не більше 5 нових за 60 с.
    Зайві не показуються: у журналі рядок `notification not shown (…)`.
- **Прогрес** — значок у **лівій** частині рядка стану: спінер або `N%`, назва розширення і назва
  завдання.
  - Підказка: `<назва> · <id>` і рядок на кожне завдання.
  - Клік відкриває меню: `Cancel <назва>` для завдань, які можна скасувати, і `Show <власник>`.
  - Після **Cancel** пункт читається `<назва>: cancelling…`, доки розширення не завершить завдання.

| Крок | Що | Так / ні, примітка |
|---|---|---|
| D1–D4 | панель, picker, годинник, декорації | |
| N1 | дві картки, рядок походження, фокус не забрано, **Open** неактивна перші 750 мс | |
| N2 | попередження зникло саме, `closed by itself` | |
| N3 | **Open** → `pressed open`; хрестик → `closed by you` | |
| N4 | прогрес у рядку стану й на панелі, +10 % за тік, `Job: done` | |
| N5 | `Cancel Gallery job` → `Job: cancelled` | |
| N6 | не більше 3 відкритих карток (якщо робили) | |

## 2. 0.3: зображення і canvas (5 хв)

| # | Що зробити | Очікується |
|---|---|---|
| I1 | Відкрийте панель **Images** (значок `image`, лівий док) | Розділи `SVG asset, plain and tinted` (логотип Fyx двічі, другий у кольорі акценту), `PNG asset in a 96×96 box: contain, cover` (та сама картинка: ціла і обрізана), `Generated at run time` (стовпчикова діаграма). |
| I2 | **Shuffle** кілька разів | Діаграма щоразу з новими стовпчиками, без миготіння панелі. |
| I3 | Розділ `A missing asset shows its alt text` | Рамка з текстом `A picture that is not in the package (this is its alt text)` і малим значком попередження. Червоного рядка помилки під ним немає. |
| I4 | Розділ `Thumbnails in list rows` | Рядки `Source files`, `Documents`, `Tests`, кожен з малим значком 16×16. |
| C1 | Відкрийте панель **Charts** (значок `table`, лівий док) | `Commits this week` і стовпчики з понеділка до неділі; під ними `Click a bar`. Наведіть курсор: підказка `Commits per day this week, Monday to Sunday`. |
| C2 | Клацніть стовпчик середи, потім над низьким стовпчиком суботи | `Wed: 42 commits` і обведення стовпчика; потім `Sat: 8 commits` (уся колонка дня над віссю рахується як влучання). Клік по підписах днів під віссю знімає вибір. |
| C3 | `Load`: подивіться 5 с, потім **Pause**, потім **Resume** | Лінія росте що пів секунди. **Pause** зупиняє її, кнопка стає **Resume**. Сховайте панель і поверніть: поки панель схована, точки не додаються. |

| Крок | Що | Так / ні, примітка |
|---|---|---|
| I1–I4 | SVG, PNG, згенероване, alt-текст, мініатюри | |
| C1–C3 | клік по стовпчику, sparkline, Pause | |

## 3. 0.3: дозволи T2 з `fyx_demo_t2` (12 хв)

Відкрийте панель **T2 capabilities** (значок `lock`, правий док). Вона має чотири розділи:
**Fetch**, **Process**, **Edit** і **Agent reviews**. Під заголовком кожного розділу стоїть рядок
стану:

| Текст | Що означає |
|---|---|
| `Permission: not asked yet` | ще не питали |
| `Permission: waiting for an answer…` | запит відкритий |
| `Permission: allowed` | дозволено (зелений) |
| `Permission: denied (change it on the extensions page)` | відмовлено (червоний) |
| `Permission: not available here` | хост цього не вміє (у цій збірці не має бути) |

**Як виглядає запит Fyx.** Діалог-попередження з кнопками **Allow** і **Don't Allow**. Внизу
рядок `T2 capabilities demo · fyx-demo-t2. You can revoke this on the extensions page.` Для
кожного розділу свій заголовок, див. таблицю.

**Порядок для кожного розділу:**
1. **Request …**. Fyx показує запит. Відповідь із таблиці.
2. Кнопка дії. Результат із таблиці.

Перші три розділи дозволяємо, а **Edit** спершу відхиляємо, щоб перевірити шлях відмови.

| # | Розділ | Що зробити | Очікується |
|---|---|---|---|
| T1 | Fetch | **Request fetch** → **Allow**. Потім **Fetch zen** | Запит `Allow T2 capabilities demo to connect to the internet?` з `Hosts: api.github.com`. Після Allow `Permission: allowed`. Fetch: спершу `Fetching…`, потім короткий афоризм GitHub. |
| T2 | Process | **Request process** → **Allow**. Потім **Run git status** | Запит `Allow T2 capabilities demo to run programs?` з переліком програм (`Programs: git …`; запишіть точний текст). Результат: `Exited with 0: the worktree is clean.` |
| T3 | Edit, відмова | **Request edit** → **Don't Allow**. Потім **Insert header** | Запит `Allow T2 capabilities demo to edit files in your projects?`. Після відмови `Permission: denied (change it on the extensions page)`. **Insert header** дає `denied: … (permission: denied)`. `README.md` не змінено: `git -C D:\fyx-user-check\t2-project diff` порожній. |
| T4 | Agent | **Request agent** → **Allow**. Потім **Start** | Запит `Allow T2 capabilities demo to watch agent sessions?`. Після **Start** кнопка стає **Stop**, видно `0 open reviews`. До цього розділу повернемося в блоці 6. |
| T5 | Повторний запит | Ще раз **Fetch zen** | Запиту більше немає, одразу результат. |
| T6 | Закрити запит без відповіді | Якщо якийсь розділ ще на `not asked yet`, натисніть його **Request …** і закрийте діалог хрестиком. Потім клацніть будь-де в панелі | Fyx нічого не запам'ятовує. Поки запит відкритий, видно `Permission: waiting for an answer…`. Після наступного кліку знову `Permission: not asked yet`, і **Request …** знову активна. |

**Відкликати й дозволити знову.**

| # | Що зробити | Очікується |
|---|---|---|
| T7 | Палітра команд: `fyx_ui: revoke UI extension permission` → `fyx-demo-t2: network:fetch` | Toast `Revoked network:fetch from fyx-demo-t2`. Рядок стану Fetch: `Permission: denied (change it on the extensions page)`. **Fetch zen** дає `denied: … (permission: denied)`. |
| T8 | `fyx_ui: open fyx extensions`, рядок **T2 capabilities demo** у **Installed** | Рядки `Network: not allowed` з **Allow**, `Runs Programs: allowed` з **Revoke**, `Edits Files: not allowed`, `Watches Agents: allowed`. **Allow** біля Network повертає дозвіл, і **Fetch zen** знову працює. «ЗАПОВНИТИ: зараз сторінка не показує dev-розширення, тож цих рядків не буде; див. "Відомі прогалини"» |
| T9 | На тій самій сторінці **Allow** біля `Edits Files`, потім у демо **Insert header**. Залежить від T8: без рядків на сторінці відмову з T3 не скасувати | `Inserted at the top of README.md; undo with Ctrl+Z.` З'являється картка `T2 capabilities demo: Insert the T2 demo header, 1 file` з **Show** і **Undo**. **Undo** прибирає рядок. Fyx нічого не зберігає сам: `README.md` лишається незбереженим. |

| Крок | Що | Так / ні, примітка |
|---|---|---|
| T1 | мережа: запит, Allow, відповідь | |
| T2 | програми: запит, Allow, `git status` | |
| T3 | правки: Don't Allow, `denied`, файл не змінено | |
| T4 | агент: запит, Allow, Start | |
| T5 | повторно без запиту | |
| T6 | закритий без відповіді запит → знову `not asked yet` | |
| T7 | відкликання командою | |
| T8 | рядки T2 на сторінці, Allow знову | |
| T9 | правка з **Show** / **Undo**, без збереження | |

## 4. Реєстр (9 хв)

**Як підготовлено.** Координатор запускає `script/fyx-test-registry` (Git Bash, cargo,
Python 3.11+) для кожної теки з блоку 0.
- Скрипт підписує реліз, `index.json`, `snapshot.json` і `blocklist.json` **тестовими** ключами, які
  ніколи нічого справжнього не підписують.
- Реліз датується «два дні тому», тож 24-годинна затримка не заважає.
- Сервера немає: реєстр — це тека, URL `file:///…`.
- Debug-збірка довіряє лише тестовим ключам. Release-збірка не довіряє жодним, і сторінка каже
  `This build trusts no registry keys yet, so it cannot install from the registry.`
- Знімок реєстру дійсний 48 год. Якщо теки старші, сторінка покаже
  `The registry's index has expired; …`. Тоді попросіть координатора перезапустити скрипт.

| # | Що зробити | Очікується |
|---|---|---|
| G1 | `fyx_ui: open fyx extensions` до будь-яких змін | Вкладка **Fyx Extensions**, рядок `The registry is off (fyx.extensions.registry_url).` |
| G2 | У `settings.json` у `"fyx": { "extensions": { … } }` додайте `"registry_url": "file:///D:/fyx-user-check/registry-1"`, збережіть. На сторінці **Refresh** | У **Available** рядок `fyx-demo-counter · 0.1.0`, `by fyx-tests`, з **Install**. Пошук `Search Fyx extensions…` за `counter` лишає лише його. |
| G3 | **Install** | Toast `Installed <name> 0.1.0`. У **Installed** рядок з `From the Fyx registry`, кнопками **Pin** і **Uninstall**. Панель Counter у доку. |
| G4 | Змініть `registry_url` на `…/registry-2`, збережіть, **Refresh** | У рядку **Update to 0.2.0** або в **Installed** кнопка **Update**. Натисніть її: `Installed …`, у рядку з'явилась **Roll Back**. |
| G5 | **Roll Back** | Toast `Rolled fyx-demo-counter back to 0.1.0`. Потім **Pin**: `Pinned fyx-demo-counter`, і в рядку `0.2.0 available; pinned`. **Unpin**. |
| G6 | Затримане оновлення T2: «ЗАПОВНИТИ: яке розширення в `registry-2` отримує новий домен або програму» | У рядку `Update <v> needs approval: Connects to <domain>` (або `Runs <program>`, `Permission tier T2`) і кнопка **Approve and Update**. Поки ви не натиснули, стара версія працює. Після натискання оновлення ставиться. |
| G7 | Змініть `registry_url` на `…/registry-3`, збережіть. **Закрийте і знову відкрийте Fyx** (блок-лист читається раз за сесію). Відкрийте сторінку | У рядку `fyx-demo-counter` примітка `Blocked (<reason>): rolls back to 0.1.0` або `Blocked (<reason>): disabled`. |
| G8 | Сповіщення про блокування | «ЗАПОВНИТИ: як його викликати». Зараз автоматичне оновлення, відкат і вимкнення роблять лише за таймером, через 6 год, і ручного запуску немає. Сповіщення має заголовок `<name> was rolled back to <v>` або `<name> was disabled`, текст `The Fyx block list says: <reason>` і кнопку **Open Fyx Extensions**. |

| Крок | Що | Так / ні, примітка |
|---|---|---|
| G1–G2 | реєстр вимкнено; після `registry_url` видно реліз | |
| G3 | встановлення з реєстру | |
| G4–G5 | оновлення, відкат, pin | |
| G6 | затримане оновлення T2 і **Approve and Update** | |
| G7 | примітка `Blocked (…)` | |
| G8 | сповіщення про блокування (якщо вдалося викликати) | |

## 5. Inspector: мережа, програми, правки (3 хв)

`developer_mode` уже ввімкнено в блоці 0.

1. `fyx_ui: toggle extension inspector`. Панель **Extension Inspector** у правому доку. Кнопка
   `fyx-demo-t2/t2`.
2. У панелі T2 натисніть **Fetch zen** і **Run git status**.
3. **Очікується** в розділі **Network, programs, edits** (найновіші зверху):
   - `SS.mmm network:fetch #N Get https://api.github.com/zen: 200 (N bytes) in N ms`;
   - `SS.mmm process:spawn #N git (2 arguments): exited 0 in N ms`;
   - з кроку T9: `workspace:edit #N "Insert the T2 demo header" (1 files): applied in N ms`;
   - для агента: `agent:observe #0 start: observing`.
4. **Очікується** в URL лише схема, хост і шлях, без запиту (`?…`) і фрагмента (`#…`).
5. Відкличте `network:fetch` (як у T7) і ще раз **Fetch zen**. **Очікується** у **Capability
   checks** рядок `… network:fetch denied: …`. У **Network, programs, edits** нового рядка
   немає, бо запит не стартував.

| Крок | Що | Так / ні, примітка |
|---|---|---|
| 3 | рядки fetch, git, edit, agent з результатом і часом | |
| 4 | URL без секретів | |
| 5 | відмова в **Capability checks** | |

## 6. Огляд змін агента: Tab і клавіатура (3 хв)

Потрібні рядки keymap W5 (`f868421052`). Відкрийте `D:\fyx-user-check\review-project`
(**File → Open Folder**, нове вікно). Увімкніть огляд: меню опцій агентної панелі → **Review Agent
Edits**. У тому самому вікні відкрийте панель T2: блок 3 був в іншому вікні, тож тут дайте
**Start** у розділі **Agent reviews**.

1. Тред з **Claude Agent** (увійдіть самостійно, як у першому дні): *In a.txt change bravo to
   BRAVO, and in b.txt change delta to DELTA. Make no other change.*
2. Toast → **Review**. Фокус у diff.

| # | Клавіші | Очікується |
|---|---|---|
| K1 | **Tab** кілька разів, потім **Shift+Tab** | Фокус іде кнопками в порядку малювання: **Accept all in turn**, **Reject all in turn**, **Reject and close**, кнопки файлів (`a.txt`, `b.txt`), потім **Accept**, **Reject**, **Accept file**, **Reject file**, **Submit**. Shift+Tab іде назад. Підказки кнопок показують їхні клавіші. |
| K2 | **Escape** з кнопки | Фокус повертається в diff. |
| K3 | **alt-j** / **alt-k**, **ctrl-k j** / **ctrl-k k** | Наступний / попередній фрагмент; наступний / попередній файл ходу. |
| K4 | **alt-c**, наберіть коментар, **Tab** | Фокус у полі `Comment`. Tab переводить фокус далі, а не вставляє табуляцію. |
| K5 | Панель T2, розділ **Agent reviews** | `1 open reviews` (або більше), рядки з файлами і станом `<n> pending`. Після рішення в огляді (**ctrl-k shift-y**, **Accept All in Turn**) стан стає `accepted`. |

| Крок | Що | Так / ні, примітка |
|---|---|---|
| K1–K2 | Tab / Shift+Tab по кнопках, Escape у diff | |
| K3 | фрагменти й файли з клавіатури | |
| K4 | Tab у полі коментаря | |
| K5 | `agent:observe` бачить огляд | |

## Відомі прогалини (станом на 2026-11-18)

Їх треба закрити до перевірки, або позначити крок як пропущений:

1. **Сповіщення й прогрес (N1–N6).** Закрито: W4 `27e6220906` додав **Notify** і **Run job**.
   Потрібна лише тека зібраної gallery 0.2 з цим комітом (блок 0, «Матеріали»).
2. **Рядки T2 на сторінці Fyx Extensions (T8, T9).** Сторінка показує в **Installed** лише
   розширення зі сховища встановлень, і лише коли ввімкнено реєстр. Тому для теки з `dev_paths`
   рядків T2 немає. Для dev-розширення немає й іншого місця, де відмову можна скасувати.
   Варіанти:
   - сторінка показує й dev-розширення (W3);
   - `fyx-demo-t2` ставиться з реєстру (див. п. 3).
3. **Реєстр для G4–G8.** `script/fyx-test-registry` робить одну версію на демо, порожній блок-лист і
   не вміє релізів T2:
   - його поділ на рівні ставить `network:fetch` у T0, а `index add` відмовляє такий реліз;
   - він не пише `network_domains` і `processes`;
   - він пакує з `extensions/<demo>`, а не з теки збірки 0.3.

   Потрібні три теки реєстру з блоку 0 (W3 або координатор).
4. **Сповіщення про блокування (G8).** Його показує лише таймер оновлень, раз на 6 год, і ручного
   запуску немає. Примітку `Blocked (…)` на сторінці видно після **Refresh**.
5. **Install на сторінці** ставить найновіший реліз з індексу, не перевіряючи 24-годинну
   затримку і блок-лист. Перевіряйте блокування через G7, а не через Install.
6. **Недовірена папка і закритий запит.** Закрито у `fyx_demo_t2` з `6bc8a1c12b`:
   - недовірена папка дає `Trust this project to …`;
   - інша відмова показує повідомлення Fyx і `(permission: <state>)`;
   - закритий без відповіді запит після наступного кліку повертається до `not asked yet`.

## Після перевірки

- Надішліть координатору таблиці результатів (або цей файл із заповненими таблицями).
- Знімки екрана помилок кладіть у `D:\fyx-user-check\shots\`.
- Профіль `D:\fyx-user-check\profile-2` можна видалити. Дозволи T2 демо лежать у ньому
  (`fyx-ui-consent.json`), тож разом з ним зникнуть і вони.
