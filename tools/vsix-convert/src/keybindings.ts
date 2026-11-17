import { own } from "./own.ts";

// VS Code keybindings (`contributes.keybindings` or a user `keybindings.json`) → Zed keymap JSON
// (`[{ context?, bindings: { "ctrl-k ctrl-c": action } }]`).
//
// Zed has no VS Code keybinding importer; its `assets/keymaps/*/vscode.json` base keymap is the
// reference for the command → action table below. Every Zed action named here is checked
// against Zed's own keymaps and action definitions by `test/zed-actions.test.ts`.

export interface VsCodeKeybinding {
  key: string;
  command: string;
  when?: string;
  args?: unknown;
  mac?: string;
  linux?: string;
  win?: string;
}

export type ZedAction = string | [string, unknown] | null;

export interface ZedKeymapSection {
  context?: string;
  bindings: Record<string, ZedAction>;
}

export type Platform = "macos" | "linux" | "windows";

/**
 * Action that hands a command to the extension sidecar. It does not exist in the host yet (W8 is
 * mocked); the integrator wires it when the sidecar lands in the editor.
 */
export const SIDECAR_COMMAND_ACTION = "fyx_vscode::RunCommand";

/** VS Code command id → Zed action. */
export const COMMAND_TO_ZED_ACTION: Readonly<Record<string, ZedAction>> = {
  "editor.action.formatDocument": "editor::Format",
  "editor.action.formatSelection": "editor::FormatSelections",
  "editor.action.commentLine": "editor::ToggleComments",
  "editor.action.blockComment": "editor::ToggleBlockComments",
  "editor.action.copyLinesDownAction": "editor::DuplicateLineDown",
  "editor.action.copyLinesUpAction": "editor::DuplicateLineUp",
  "editor.action.moveLinesDownAction": "editor::MoveLineDown",
  "editor.action.moveLinesUpAction": "editor::MoveLineUp",
  "editor.action.deleteLines": "editor::DeleteLine",
  "editor.action.insertLineAfter": "editor::NewlineBelow",
  "editor.action.insertLineBefore": "editor::NewlineAbove",
  "editor.action.indentLines": "editor::Indent",
  "editor.action.outdentLines": "editor::Outdent",
  "editor.action.addSelectionToNextFindMatch": ["editor::SelectNext", { replace_newest: false }],
  "editor.action.moveSelectionToNextFindMatch": ["editor::SelectNext", { replace_newest: true }],
  "editor.action.selectHighlights": "editor::SelectAllMatches",
  "editor.action.insertCursorAbove": "editor::AddSelectionAbove",
  "editor.action.insertCursorBelow": "editor::AddSelectionBelow",
  "editor.action.insertCursorAtEndOfEachLineSelected": "editor::SplitSelectionIntoLines",
  "editor.action.revealDefinition": "editor::GoToDefinition",
  "editor.action.revealDefinitionAside": "editor::GoToDefinitionSplit",
  "editor.action.goToReferences": "editor::FindAllReferences",
  "editor.action.goToImplementation": "editor::GoToImplementation",
  "editor.action.goToTypeDefinition": "editor::GoToTypeDefinition",
  "editor.action.rename": "editor::Rename",
  "editor.action.quickFix": "editor::ToggleCodeActions",
  "editor.action.showHover": "editor::Hover",
  "editor.action.triggerSuggest": "editor::ShowCompletions",
  "editor.action.triggerParameterHints": "editor::ShowSignatureHelp",
  "editor.action.marker.next": "editor::GoToDiagnostic",
  "editor.action.marker.prev": "editor::GoToPreviousDiagnostic",
  "editor.action.jumpToBracket": "editor::MoveToEnclosingBracket",
  "editor.action.smartSelect.expand": "editor::SelectLargerSyntaxNode",
  "editor.action.smartSelect.shrink": "editor::SelectSmallerSyntaxNode",
  "editor.action.toggleWordWrap": "editor::ToggleSoftWrap",
  "editor.action.transformToUppercase": "editor::ConvertToUpperCase",
  "editor.action.transformToLowercase": "editor::ConvertToLowerCase",
  "editor.action.transformToTitlecase": "editor::ConvertToTitleCase",
  "editor.action.sortLinesAscending": "editor::SortLinesCaseSensitive",
  "editor.action.joinLines": "editor::JoinLines",
  "editor.action.selectAll": "editor::SelectAll",
  "editor.action.clipboardCutAction": "editor::Cut",
  "editor.action.clipboardCopyAction": "editor::Copy",
  "editor.action.clipboardPasteAction": "editor::Paste",
  "editor.action.startFindReplaceAction": "buffer_search::DeployReplace",
  "editor.action.nextMatchFindAction": "search::SelectNextMatch",
  "editor.action.previousMatchFindAction": "search::SelectPreviousMatch",
  "editor.fold": "editor::Fold",
  "editor.unfold": "editor::UnfoldLines",
  "editor.foldAll": "editor::FoldAll",
  "editor.unfoldAll": "editor::UnfoldAll",
  "actions.find": "buffer_search::Deploy",
  undo: "editor::Undo",
  redo: "editor::Redo",
  cursorUndo: "editor::UndoSelection",
  cursorHome: "editor::MoveToBeginningOfLine",
  cursorEnd: "editor::MoveToEndOfLine",
  cursorTop: "editor::MoveToBeginning",
  cursorBottom: "editor::MoveToEnd",
  cursorWordLeft: "editor::MoveToPreviousWordStart",
  cursorWordEndRight: "editor::MoveToNextWordEnd",
  deleteWordLeft: "editor::DeleteToPreviousWordStart",
  deleteWordRight: "editor::DeleteToNextWordEnd",
  expandLineSelection: "editor::SelectLine",
  "workbench.action.quickOpen": "file_finder::Toggle",
  "workbench.action.showCommands": "command_palette::Toggle",
  "workbench.action.gotoLine": "go_to_line::Toggle",
  "workbench.action.gotoSymbol": "outline::Toggle",
  "workbench.action.showAllSymbols": "project_symbols::Toggle",
  "workbench.action.files.save": "workspace::Save",
  "workbench.action.files.saveAs": "workspace::SaveAs",
  "workbench.action.files.saveAll": "workspace::SaveAll",
  "workbench.action.files.newUntitledFile": "workspace::NewFile",
  "workbench.action.closeActiveEditor": "pane::CloseActiveItem",
  "workbench.action.closeAllEditors": "pane::CloseAllItems",
  "workbench.action.reopenClosedEditor": "pane::ReopenClosedItem",
  "workbench.action.nextEditor": "pane::ActivateNextItem",
  "workbench.action.previousEditor": "pane::ActivatePreviousItem",
  "workbench.action.splitEditor": "pane::SplitRight",
  "workbench.action.navigateBack": "pane::GoBack",
  "workbench.action.navigateForward": "pane::GoForward",
  "workbench.action.toggleSidebarVisibility": "workspace::ToggleLeftDock",
  "workbench.action.togglePanel": "workspace::ToggleBottomDock",
  "workbench.action.terminal.toggleTerminal": "terminal_panel::Toggle",
  "workbench.action.terminal.new": "workspace::NewTerminal",
  "workbench.view.explorer": "project_panel::ToggleFocus",
  "workbench.action.findInFiles": "pane::DeploySearch",
  "workbench.view.search": "pane::DeploySearch",
  "workbench.actions.view.problems": "diagnostics::Deploy",
  "workbench.action.openSettings": "zed::OpenSettings",
  "workbench.action.openGlobalKeybindings": "zed::OpenKeymap",
  "workbench.action.zoomIn": "zed::IncreaseBufferFontSize",
  "workbench.action.zoomOut": "zed::DecreaseBufferFontSize",
  "workbench.action.zoomReset": "zed::ResetBufferFontSize",
  "workbench.action.toggleZenMode": "workspace::ToggleCenteredLayout",
  "workbench.action.toggleFullScreen": "zed::ToggleFullScreen",
  "workbench.action.newWindow": "workspace::NewWindow",
  "workbench.action.openRecent": "projects::OpenRecent",
  "workbench.action.quit": "zed::Quit",
};

/** VS Code language id → file extensions, for `editorLangId == x` (Zed's editor context knows `extension`, not language). */
const LANGUAGE_EXTENSIONS: Readonly<Record<string, readonly string[]>> = {
  javascript: ["js", "mjs", "cjs"],
  javascriptreact: ["jsx"],
  typescript: ["ts", "mts", "cts"],
  typescriptreact: ["tsx"],
  python: ["py"],
  rust: ["rs"],
  go: ["go"],
  c: ["c", "h"],
  cpp: ["cpp", "cc", "hpp"],
  csharp: ["cs"],
  java: ["java"],
  json: ["json"],
  jsonc: ["jsonc"],
  markdown: ["md"],
  yaml: ["yaml", "yml"],
  html: ["html", "htm"],
  css: ["css"],
  scss: ["scss"],
  shellscript: ["sh", "bash"],
  ruby: ["rb"],
  php: ["php"],
  toml: ["toml"],
  xml: ["xml"],
  csv: ["csv"],
  plaintext: ["txt"],
};

/** VS Code context key → Zed context expression. */
const CONTEXT_KEYS: Readonly<Record<string, string>> = {
  editorTextFocus: "Editor",
  editorFocus: "Editor",
  textInputFocus: "Editor",
  editorIsOpen: "Editor",
  inputFocus: "Editor",
  terminalFocus: "Terminal",
  terminalIsOpen: "Terminal",
  inQuickOpen: "Picker",
  inQuickInput: "Picker",
  suggestWidgetVisible: "showing_completions",
  explorerViewletFocus: "ProjectPanel",
  filesExplorerFocus: "ProjectPanel",
  explorerViewletVisible: "ProjectPanel",
  searchViewletFocus: "ProjectSearchBar",
  inSnippetMode: "in_snippet",
  hasNextTabstop: "has_next_tabstop",
  hasPrevTabstop: "has_previous_tabstop",
  editorHasMultipleSelections: "multiple_selections",
};

export interface KeymapResult {
  keymap: ZedKeymapSection[];
  warnings: string[];
}

export interface ConvertKeybindingsOptions {
  platform?: Platform;
  /** Commands contributed by the extension itself; they route to the sidecar. */
  extensionCommands?: ReadonlySet<string>;
}

export function convertKeybindings(
  bindings: readonly VsCodeKeybinding[],
  options: ConvertKeybindingsOptions = {},
): KeymapResult {
  const platform = options.platform ?? hostPlatform();
  const extensionCommands = options.extensionCommands ?? new Set<string>();
  const warnings: string[] = [];
  const sections = new Map<string, ZedKeymapSection>();

  for (const binding of bindings) {
    const rawKey = (platform === "macos" ? binding.mac : platform === "linux" ? binding.linux : binding.win) ?? binding.key;
    if (!rawKey || !binding.command) continue;
    const label = `${rawKey} → ${binding.command}`;

    const keys = convertKey(rawKey, platform);
    if (typeof keys !== "string") {
      warnings.push(`${label}: ${keys.error}, skipped`);
      continue;
    }

    const removal = binding.command.startsWith("-");
    const command = removal ? binding.command.slice(1) : binding.command;
    let action: ZedAction;
    if (Object.hasOwn(COMMAND_TO_ZED_ACTION, command)) {
      action = COMMAND_TO_ZED_ACTION[command];
      if (binding.args !== undefined && action !== null) {
        warnings.push(`${label}: args dropped, Zed action ${actionName(action)} takes its own`);
      }
    } else if (extensionCommands.has(command)) {
      action = [SIDECAR_COMMAND_ACTION, binding.args === undefined ? { command } : { command, args: binding.args }];
    } else {
      warnings.push(`${label}: no Zed equivalent for command, skipped`);
      continue;
    }
    const target = action;
    if (removal) action = null;

    const contextResult = convertWhen(binding.when);
    for (const w of contextResult.warnings) warnings.push(`${label}: ${w}`);
    let context = contextResult.context;
    if (context === undefined && target !== null && actionName(target).startsWith("editor::")) {
      context = "Editor";
    }
    const sectionKey = context ?? "";
    let section = sections.get(sectionKey);
    if (!section) {
      section = context === undefined ? { bindings: {} } : { context, bindings: {} };
      sections.set(sectionKey, section);
    }
    if (keys in section.bindings) warnings.push(`${label}: overrides an earlier binding for ${keys}`);
    section.bindings[keys] = action;
  }
  return { keymap: [...sections.values()], warnings };
}

function actionName(action: Exclude<ZedAction, null>): string {
  return typeof action === "string" ? action : action[0];
}

function hostPlatform(): Platform {
  return process.platform === "darwin" ? "macos" : process.platform === "win32" ? "windows" : "linux";
}

const KEY_NAMES: Readonly<Record<string, string>> = {
  esc: "escape",
  escape: "escape",
  enter: "enter",
  return: "enter",
  tab: "tab",
  space: "space",
  backspace: "backspace",
  delete: "delete",
  insert: "insert",
  home: "home",
  end: "end",
  pageup: "pageup",
  pagedown: "pagedown",
  up: "up",
  down: "down",
  left: "left",
  right: "right",
};

const MODIFIERS: Readonly<Record<string, Record<Platform, string>>> = {
  ctrl: { macos: "ctrl", linux: "ctrl", windows: "ctrl" },
  shift: { macos: "shift", linux: "shift", windows: "shift" },
  alt: { macos: "alt", linux: "alt", windows: "alt" },
  cmd: { macos: "cmd", linux: "super", windows: "win" },
  meta: { macos: "cmd", linux: "super", windows: "win" },
  win: { macos: "cmd", linux: "super", windows: "win" },
};

/** `ctrl+shift+k ctrl+c` → `ctrl-shift-k ctrl-c`. */
export function convertKey(key: string, platform: Platform): string | { error: string } {
  const chords = key.trim().toLowerCase().split(/\s+/);
  const out: string[] = [];
  for (const chord of chords) {
    // `ctrl++` means ctrl and the plus key.
    const parts = chord.endsWith("++") ? [...chord.slice(0, -2).split("+"), "+"] : chord.split("+");
    const keyName = parts.pop();
    if (!keyName) return { error: `empty key in "${key}"` };
    const mods: string[] = [];
    for (const m of parts) {
      const mapped = own(MODIFIERS, m)?.[platform];
      if (!mapped) return { error: `unknown modifier "${m}"` };
      mods.push(mapped);
    }
    let name = own(KEY_NAMES, keyName);
    if (!name) {
      if (/^f([1-9]|1[0-9]|2[0-4])$/.test(keyName) || /^[a-z0-9]$/.test(keyName) || /^[`\-=[\]\\;',./+]$/.test(keyName)) {
        name = keyName;
      } else {
        return { error: `key "${keyName}" has no Zed name` };
      }
    }
    out.push([...mods, name].join("-"));
  }
  return out.join(" ");
}

// ---------------------------------------------------------------------------------------------
// `when` clauses → Zed contexts

type When =
  | { kind: "true" }
  | { kind: "atom"; zed: string }
  | { kind: "not"; expr: When }
  | { kind: "and" | "or"; left: When; right: When };

const TRUE: When = { kind: "true" };

export function convertWhen(when: string | undefined): { context?: string; warnings: string[] } {
  const warnings: string[] = [];
  if (!when || !when.trim()) return { warnings };
  let expr: When;
  try {
    expr = new WhenParser(when, warnings).parse();
  } catch (err) {
    warnings.push(`when "${when}" not parsed (${(err as Error).message}), context dropped`);
    return { warnings };
  }
  return expr.kind === "true" ? { warnings } : { context: render(expr, 0), warnings };
}

function render(e: When, parentPrec: number): string {
  switch (e.kind) {
    case "true":
      return "";
    case "atom":
      return e.zed;
    case "not":
      return `!${render(e.expr, 3)}`;
    case "and":
    case "or": {
      const prec = e.kind === "and" ? 2 : 1;
      const s = `${render(e.left, prec)} ${e.kind === "and" ? "&&" : "||"} ${render(e.right, prec)}`;
      return prec < parentPrec ? `(${s})` : s;
    }
  }
}

class WhenParser {
  private tokens: string[];
  private pos = 0;
  private source: string;
  private warnings: string[];

  constructor(source: string, warnings: string[]) {
    this.source = source;
    this.warnings = warnings;
    this.tokens = source.match(/\s*(&&|\|\||!=|==|=~|<=|>=|!|\(|\)|<|>|'[^']*'|"[^"]*"|\/(?:\\.|[^/])+\/[a-z]*|[^\s()!=<>&|]+)/g)?.map((t) => t.trim()) ?? [];
    if (this.tokens.join("").replace(/\s/g, "") !== source.replace(/\s/g, "")) throw new Error("unexpected characters");
  }

  parse(): When {
    const e = this.or();
    if (this.pos < this.tokens.length) throw new Error(`unexpected "${this.tokens[this.pos]}"`);
    return e;
  }

  private peek(): string | undefined {
    return this.tokens[this.pos];
  }

  private or(): When {
    let left = this.and();
    while (this.peek() === "||") {
      this.pos++;
      const right = this.and();
      left = left.kind === "true" || right.kind === "true" ? TRUE : { kind: "or", left, right };
    }
    return left;
  }

  private and(): When {
    let left = this.unary();
    while (this.peek() === "&&") {
      this.pos++;
      const right = this.unary();
      left = left.kind === "true" ? right : right.kind === "true" ? left : { kind: "and", left, right };
    }
    return left;
  }

  private unary(): When {
    if (this.peek() === "!") {
      this.pos++;
      const inner = this.unary();
      if (inner.kind === "true") return TRUE;
      return { kind: "not", expr: inner };
    }
    if (this.peek() === "(") {
      this.pos++;
      const e = this.or();
      if (this.peek() !== ")") throw new Error("missing )");
      this.pos++;
      return e;
    }
    return this.comparison();
  }

  private comparison(): When {
    const key = this.tokens[this.pos++];
    if (key === undefined) throw new Error("unexpected end");
    const op = this.peek();
    if (op === "==" || op === "!=" || op === "=~" || op === "<" || op === ">" || op === "<=" || op === ">=") {
      this.pos++;
      const value = unquote(this.tokens[this.pos++] ?? "");
      return this.mapComparison(key, op, value);
    }
    if (op === "in" || (op === "not" && this.tokens[this.pos + 1] === "in")) {
      this.pos += op === "in" ? 2 : 3;
      return this.drop(`"${key} ${op} …"`);
    }
    if (key === "true") return TRUE;
    if (key === "false") return this.drop("literal false");
    const mapped = own(CONTEXT_KEYS, key);
    if (mapped) return { kind: "atom", zed: mapped };
    return this.drop(`context key "${key}"`);
  }

  private mapComparison(key: string, op: string, value: string): When {
    if ((key === "editorLangId" || key === "resourceLangId") && (op === "==" || op === "!=")) {
      const exts = own(LANGUAGE_EXTENSIONS, value);
      if (!exts) return this.drop(`${key} == ${value} (no known file extension)`);
      this.warnings.push(`${key} == ${value} approximated by file extension (${exts.join(", ")})`);
      const any = exts.map<When>((e) => ({ kind: "atom", zed: `extension == ${e}` })).reduce((l, r) => ({ kind: "or", left: l, right: r }));
      return op === "==" ? any : { kind: "not", expr: any };
    }
    if (key === "resourceExtname" && (op === "==" || op === "!=")) {
      const atom: When = { kind: "atom", zed: `extension == ${value.replace(/^\./, "")}` };
      return op === "==" ? atom : { kind: "not", expr: atom };
    }
    return this.drop(`"${key} ${op} ${value}"`);
  }

  private drop(what: string): When {
    this.warnings.push(`${what} has no Zed context, dropped (binding is broader than in VS Code)`);
    return TRUE;
  }
}

function unquote(s: string): string {
  return /^(['"]).*\1$/.test(s) ? s.slice(1, -1) : s;
}
