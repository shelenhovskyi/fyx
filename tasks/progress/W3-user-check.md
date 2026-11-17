# W3: ручна перевірка plan 1.1–1.3 і згоди (consent)

Одна перевірка на все: встановлення з `.fyxext`, оновлення, відкат (rollback), закріплення (pin),
dev-встановлення, hot reload і **Restart extension**, а також запит згоди. Назви команд, кнопок і
повідомлень англійською, як у Fyx.

Потрібна збірка `fyx.exe` з гілки, де вже є коміти W3 `d74c8bb2ac` (1.1), `db6619b9f8` (1.2) і
коміт 1.3 (Restart extension).

## Підготовка

1. Переконайтеся, що змінна середовища `FYX_UI_DEV_EXTENSIONS` **не** задана: інакше demo-розширення
   завантажаться з неї і займуть ті самі id.
2. Чистий стан (якщо Fyx уже запускали з розширеннями): закрийте Fyx і видаліть, якщо вони є,
   `%LOCALAPPDATA%\Fyx\extensions\fyx` і `%LOCALAPPDATA%\Fyx\fyx-ui-consent.json`.
3. Матеріали для перевірки лежать у `D:\fyx-user-check\w3`:
   - `packages\fyx-demo-counter-0.1.0.fyxext` і `packages\fyx-demo-counter-0.2.0.fyxext`: той самий
     Counter demo з різними версіями;
   - `dev-todo\`: Todo demo як dev-папка;
   - `crash-item\`: елемент рядка стану з кнопками **Ping** і **Crash**.

   Щоб створити їх заново (з кореня репозиторію, Git Bash):

   ```bash
   script/fyx-build-demos --release
   ```

   ```bash
   python script/fyx-pack-ui-extension.py extensions/fyx_demo_counter --out D:/fyx-user-check/w3/packages
   ```

   ```bash
   python script/fyx-pack-ui-extension.py extensions/fyx_demo_counter --version 0.2.0 --out D:/fyx-user-check/w3/packages
   ```

   Папку `crash-item` складають `crates/ui_ext_wasm/fixtures/status_item/extension.toml` і
   зібраний тестами `fyx-ui-fixtures/wasm32-wasip2/release/fyx_ui_test_status_item.wasm`,
   перейменований на `fyx_ui.wasm`.
4. Запустіть Fyx і відкрийте будь-яку папку. Команди нижче запускаються з палітри команд
   (**Ctrl+Shift+P**).

## 1. Встановлення з `.fyxext`

1. Команда `fyx_ui: install UI extension from file`. Виберіть
   `D:\fyx-user-check\w3\packages\fyx-demo-counter-0.1.0.fyxext`.
2. **Очікується** діалог **Install fyx-demo-counter 0.1.0?**, у ньому `ui:panel: add dock panels`
   і `Unsigned, not from the registry`. Натисніть **Install**.
3. **Очікується** сповіщення `Installed fyx-demo-counter 0.1.0` і нова кнопка панелі Counter у
   доку. Відкрийте панель, натисніть **Increment** кілька разів.
4. Окремого запиту **Enable** бути **не** повинно: згоду дано в діалозі встановлення.

## 2. Оновлення

1. Знову `fyx_ui: install UI extension from file`, тепер `fyx-demo-counter-0.2.0.fyxext`.
2. **Очікується** в діалозі рядок `Installed now: 0.1.0`. Натисніть **Install**.
3. **Очікується** сповіщення `Installed fyx-demo-counter 0.2.0 (previous: 0.1.0)`. Панель лишається
   відкритою на своєму місці, лічильник знову 0 (нова версія змонтувалася).

## 3. Відкат

1. Команда `fyx_ui: roll back UI extension`. **Очікується** діалог з рядком
   `fyx-demo-counter: 0.2.0 → 0.1.0`; натисніть **fyx-demo-counter**.
2. **Очікується** сповіщення `Rolled fyx-demo-counter back to 0.1.0`, панель перемонтувалась
   (лічильник 0).

## 4. Закріплення версії

1. Команда `fyx_ui: toggle pin UI extension`, виберіть **fyx-demo-counter**. **Очікується**
   `Pinned fyx-demo-counter to 0.1.0`.
2. Спробуйте встановити `fyx-demo-counter-0.2.0.fyxext`. **Очікується** помилка з текстом
   `fyx-demo-counter is pinned to 0.1.0; unpin it to install 0.2.0`; версія не змінилась.
3. Ще раз `fyx_ui: toggle pin UI extension`. **Очікується** `Unpinned fyx-demo-counter`.

## 5. Після перезапуску

1. Закрийте Fyx і відкрийте знову.
2. **Очікується**, що панель Counter на місці без жодних діалогів і без запиту **Enable**.

## 6. Dev-встановлення

1. Команда `fyx_ui: install dev UI extension`, виберіть папку `D:\fyx-user-check\w3\dev-todo`.
2. **Очікується** діалог **Install the dev UI extension fyx-demo-todo?** з можливостями і шляхом до
   папки. Натисніть **Install**.
3. **Очікується** сповіщення `Installed the dev UI extension fyx-demo-todo` і панель Todo.
4. Відкрийте налаштування (**Ctrl+,**). **Очікується** запис:

   ```json
   "fyx": { "extensions": { "dev_paths": ["D:\\fyx-user-check\\w3\\dev-todo"] } }
   ```

## 7. Hot reload

1. У панелі Todo змініть щось (додайте пункт або позначте виконаним).
2. У PowerShell «перезберіть» компонент, оновивши час зміни файлу:

   ```powershell
   (Get-Item D:\fyx-user-check\w3\dev-todo\fyx_ui.wasm).LastWriteTime = Get-Date
   ```

3. **Очікується** приблизно за 1 секунду: панель Todo перемонтувалась (зміни з кроку 1 зникли),
   панель лишилась відкритою там, де була, редактор не перезапускався.

## 8. Невдале перезавантаження

1. Зіпсуйте компонент (копія збережеться):

   ```powershell
   Copy-Item D:\fyx-user-check\w3\dev-todo\fyx_ui.wasm D:\fyx-user-check\w3\fyx_ui.wasm.bak
   ```

   ```powershell
   Set-Content D:\fyx-user-check\w3\dev-todo\fyx_ui.wasm "broken"
   ```

2. **Очікується**: панель Todo **працює далі** (стара версія), унизу панелі червоний рядок
   `Reload failed: …` і кнопка **Retry**.
3. Поверніть компонент:

   ```powershell
   Copy-Item D:\fyx-user-check\w3\fyx_ui.wasm.bak D:\fyx-user-check\w3\dev-todo\fyx_ui.wasm -Force
   ```

4. **Очікується**: панель сама перезавантажилась, рядок помилки зник.

## 9. Запит згоди

1. У налаштуваннях (**Ctrl+,**) допишіть до `dev_paths` другу папку вручну і збережіть:

   ```json
   "fyx": { "extensions": { "dev_paths": ["D:\\fyx-user-check\\w3\\dev-todo", "D:\\fyx-user-check\\w3\\crash-item"] } }
   ```

2. **Очікується** сповіщення: `The UI extension fyx-test-status-item is installed but not enabled.
   It asks for: ui:status-bar: add status bar items. Folder: …` з кнопками **Enable** і
   **Not now**. Нічого нового в рядку стану поки немає.
3. Натисніть **Enable**. **Очікується** праворуч у рядку стану: дзвіночок, `Pings: 0`, **Ping**,
   **Crash**.

## 10. Restart extension

1. Натисніть **Ping**: `Pings: 1`.
2. Натисніть **Crash** (розширення навмисно падає). **Очікується** замість елемента червоний текст
   `fyx-test-status-item stopped: …` і кнопка **Restart extension**. Решта Fyx працює.
3. Натисніть **Restart extension**. **Очікується** знову `Pings: 0`, **Ping** працює.
4. Команда `fyx_ui: restart UI extension`, виберіть **fyx-demo-todo**. **Очікується**: панель Todo
   перемонтувалась.

## 11. Прибирання

1. Команда `fyx_ui: remove dev UI extension`, виберіть **dev-todo**, потім так само **crash-item**.
   **Очікується**: їхня панель і елемент рядка стану показують `… was uninstalled`, записи зникли з
   `dev_paths`.
2. Команда `fyx_ui: uninstall UI extension`, виберіть **fyx-demo-counter**. **Очікується**
   `Uninstalled fyx-demo-counter`, панель показує `fyx-demo-counter was uninstalled`; після
   перезапуску її немає.

## Результати

| Крок | Що | Так / ні, примітка |
|---|---|---|
| 1 | встановлення з `.fyxext`, без запиту Enable | |
| 2 | оновлення до 0.2.0 | |
| 3 | відкат до 0.1.0 | |
| 4 | закріплення відмовляє 0.2.0, відкріплення | |
| 5 | після перезапуску на місці, без запитів | |
| 6 | dev-встановлення, запис у `dev_paths` | |
| 7 | hot reload приблизно за 1 с, панель на місці | |
| 8 | `Reload failed` зі старою версією, самовідновлення | |
| 9 | запит Enable для доданої вручну папки | |
| 10 | Crash → Restart extension; команда restart | |
| 11 | remove dev, uninstall | |
