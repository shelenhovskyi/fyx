# vsix-convert

Converts the declarative parts of a VS Code extension (a `.vsix` or an unpacked folder) into
Zed/Fyx formats:

| VS Code contribution | Output | Zed reads it as |
|---|---|---|
| `contributes.themes` (color themes) | `themes/<id>.json` (theme family, schema v0.2.0) | an extension's `themes/` folder, or `~/.config/fyx/themes/` |
| `contributes.snippets` / `.code-snippets` | `snippets/<scope>.json` + `snippets = [...]` in `extension.toml` | an extension's snippets, or `~/.config/fyx/snippets/` |
| `contributes.keybindings` | `keymap.json` | a fragment to merge into the user keymap (Zed extensions cannot ship keymaps) |

Anything it cannot carry over is listed in `CONVERSION-WARNINGS.txt` next to the output.

```bash
npm install
node src/cli.ts path/to/extension.vsix -o out
```

`--platform macos|linux|windows` picks the `mac`/`linux`/`win` key variants (default: this OS).

## Themes: reused rules from `crates/theme_importer`

`src/theme.ts` and `src/syntax.ts` are a port of `crates/theme_importer/src/vscode/{converter,syntax}.rs`:
the VS Code color id → Zed key tables (`convert_theme_colors`, `convert_status_colors`), the syntax
token → TextMate scope lists (`ZedSyntaxToken::to_vscode`), the ranking (`rank_match`: earlier scopes
weigh more, comma-separated scopes are split, exact string match only), the fallback chain
(`fallbacks`), the `hint` default `#969696ff`, and `fontStyle` → `font_style`/`font_weight`.
A tie in ranking goes to the later rule, as Rust's `Iterator::max_by_key` does.

### Differences from theme_importer

| Topic | theme_importer (Rust CLI) | vsix-convert |
|---|---|---|
| Appearance | always `dark` | from `uiTheme` (`vs`, `vs-dark`, `hc-black`, `hc-light`) or the theme's `type` |
| Input | one theme JSON | a whole extension: every contributed theme becomes one family |
| `include` (base theme) | ignored | followed; child colours and rules override the base |
| `tokenColors` as a path | parse error | the referenced JSON file is loaded (`.tmTheme` is reported, not converted) |
| Colour values | passed through verbatim | normalized to `#rrggbbaa`; invalid values are skipped with a warning, so the next fallback id applies |
| Missing values | serialized as `null` | omitted |
| Comments / trailing commas | `serde_json_lenient` | an equivalent JSONC reader |
| `package.nls.json` | n/a | `%placeholders%` in labels are resolved |

Not converted by either: `semanticTokenColors`, accents, player colours.

## Snippets

Zed reads VS Code's snippet JSON directly (`crates/snippet_provider/src/format.rs`), so the converter
only fixes what Zed would reject or misfile:

- Files are named by Zed's `snippet_scope_id()` (lowercased Zed language name): `typescriptreact` →
  `tsx`, `shellscript` → `shell script`, `cpp` → `c++`, `javascriptreact` → `javascript`. Languages
  Zed does not ship are kept and flagged.
- `.code-snippets` files are split by each snippet's `scope`; no scope means `snippets.json` (global).
- Zed's body parser (`crates/snippet/src/snippet.rs`) only accepts `$n`, `${n}`, `${n:…}`, `${n|a,b|}`
  and drops a snippet with variables. Variables become placeholders the way VS Code treats unknown
  ones (`${TM_FILENAME}` → `${2:TM_FILENAME}`, `${VAR:default}` → `${2:default}`); transforms are
  removed. Every rewritten body is checked against a port of Zed's parser.
- `scope` and `isFileTemplate` are dropped.

## Keybindings

Zed has no VS Code keybinding importer; its VS Code base keymap (`assets/keymaps/*/vscode.json`) is
the reference for the command table in `src/keybindings.ts`. `test/zed-actions.test.ts` checks every
Zed action in that table against this checkout's keymaps and `actions!` definitions.

- Keys: `ctrl+shift+k ctrl+c` → `ctrl-shift-k ctrl-c`; `cmd`/`meta`/`win` map to the platform
  modifier. Numpad and `oem_*` keys have no Zed name and are skipped.
- Commands: 99 built-in VS Code commands map to Zed actions; the extension's own commands map to
  `fyx_vscode::RunCommand` (routes to the VS Code sidecar; the action is not in the host yet);
  everything else is skipped and listed.
- `when` clauses are parsed (`&&`, `||`, `!`, `==`, `!=`, `=~`, `in`, parentheses) and mapped to Zed
  contexts (`editorTextFocus` → `Editor`, `terminalFocus` → `Terminal`, `suggestWidgetVisible` →
  `showing_completions`, `editorLangId == x` → `extension == …`). A clause with no Zed equivalent is
  dropped, which makes the binding broader than in VS Code; each drop is a warning.
- `-command` removals become `null` bindings.

Measured on k--kato.intellij-idea-keybindings: its 220 entries yield 63 Zed bindings; 136 entries
are skipped, most of them for VS Code commands Zed has no action for (see docs/vscode-coverage.md).

## Tests

```bash
npm test
```
