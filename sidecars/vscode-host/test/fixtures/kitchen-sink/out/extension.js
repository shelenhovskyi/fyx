// Test fixture: a plain CommonJS VS Code extension that touches every API the Fyx shim covers.
"use strict";
const vscode = require("vscode");

class Tree {
  constructor() {
    this.emitter = new vscode.EventEmitter();
    this.onDidChangeTreeData = this.emitter.event;
    this.extra = false;
  }
  getChildren(element) {
    if (!element) return ["fruits", "veg", ...(this.extra ? ["nuts"] : [])];
    if (element === "fruits") return ["apple", "pear"];
    return [];
  }
  getTreeItem(element) {
    const parent = element === "fruits" || element === "veg";
    const item = new vscode.TreeItem(element, parent ? vscode.TreeItemCollapsibleState.Collapsed : vscode.TreeItemCollapsibleState.None);
    item.iconPath = new vscode.ThemeIcon(parent ? "folder" : "symbol-misc");
    if (!parent) {
      item.description = "leaf";
      item.command = { command: "sink.hello", title: "Hello", arguments: [element] };
    }
    return item;
  }
}

function activate(context) {
  const out = vscode.window.createOutputChannel("Sink");
  out.appendLine("activated");
  console.log("kitchen-sink activated");

  const status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  status.text = "$(check) Sink";
  status.command = "sink.hello";
  status.show();
  context.subscriptions.push(status);

  const tree = new Tree();
  const view = vscode.window.createTreeView("sinkTree", { treeDataProvider: tree });
  context.subscriptions.push(view);

  const diagnostics = vscode.languages.createDiagnosticCollection("sink");
  const decoration = vscode.window.createTextEditorDecorationType({
    backgroundColor: new vscode.ThemeColor("editorWarning.background"),
    color: "#ff0000",
  });

  const lint = (doc) => {
    if (doc.languageId !== "plaintext") return;
    const found = [];
    for (let line = 0; line < doc.lineCount; line++) {
      const text = doc.lineAt(line).text;
      const ix = text.indexOf("TODO");
      if (ix >= 0) found.push(new vscode.Diagnostic(new vscode.Range(line, ix, line, ix + 4), "TODO found", vscode.DiagnosticSeverity.Warning));
    }
    diagnostics.set(doc.uri, found);
  };
  context.subscriptions.push(vscode.workspace.onDidOpenTextDocument(lint), vscode.workspace.onDidChangeTextDocument((e) => lint(e.document)));
  vscode.workspace.textDocuments.forEach(lint);

  context.subscriptions.push(
    vscode.window.onDidChangeActiveTextEditor((editor) => {
      if (!editor) return;
      const ranges = diagnostics.get(editor.document.uri)?.map((d) => d.range) ?? [];
      // Options objects with a null hoverMessage, as indent-rainbow sends them.
      editor.setDecorations(decoration, ranges.map((range) => ({ range, hoverMessage: null })));
    }),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("sink.hello", async (name) => {
      const greeting = vscode.workspace.getConfiguration("sink").get("greeting");
      out.appendLine(`${greeting}, ${name ?? "world"}`);
      status.text = `$(check) ${name ?? "world"}`;
      tree.extra = true;
      tree.emitter.fire();
      return `${greeting}, ${name ?? "world"}`;
    }),
    vscode.commands.registerCommand("sink.pick", async () => {
      const picked = await vscode.window.showQuickPick(["alpha", "beta", { label: "gamma", description: "third" }], { placeHolder: "Pick one" });
      out.appendLine(`picked ${typeof picked === "string" ? picked : picked?.label}`);
      return typeof picked === "string" ? picked : picked?.label;
    }),
    vscode.commands.registerCommand("sink.ask", async () => {
      const answer = await vscode.window.showInputBox({ prompt: "Name?", validateInput: (v) => (v.length < 2 ? "too short" : undefined) });
      const choice = await vscode.window.showInformationMessage(`Hi ${answer}`, "OK", "Cancel");
      return { answer, choice };
    }),
    vscode.commands.registerTextEditorCommand("sink.upper", (editor, edit) => {
      edit.replace(new vscode.Range(0, 0, 0, editor.document.lineAt(0).text.length), editor.document.lineAt(0).text.toUpperCase());
      editor.selection = new vscode.Selection(0, 0, 0, 5);
    }),
    vscode.commands.registerCommand("sink.webview", () => vscode.window.createWebviewPanel("x", "X", vscode.ViewColumn.One, {})),
    vscode.commands.registerCommand("sink.fs", async () => {
      const root = vscode.workspace.workspaceFolders[0].uri;
      const file = vscode.Uri.joinPath(root, "sub", "note.txt");
      await vscode.workspace.fs.writeFile(file, new TextEncoder().encode("written by sink"));
      const back = new TextDecoder().decode(await vscode.workspace.fs.readFile(file));
      const found = await vscode.workspace.findFiles("**/*.txt");
      const stat = await vscode.workspace.fs.stat(file);
      return { back, found: found.map((u) => vscode.workspace.asRelativePath(u)).sort(), isFile: stat.type === vscode.FileType.File };
    }),
    vscode.languages.registerDocumentFormattingEditProvider("plaintext", {
      provideDocumentFormattingEdits(doc) {
        const edits = [];
        for (let line = 0; line < doc.lineCount; line++) {
          const text = doc.lineAt(line).text;
          const trimmed = text.replace(/\s+$/, "");
          if (trimmed !== text) edits.push(vscode.TextEdit.delete(new vscode.Range(line, trimmed.length, line, text.length)));
        }
        return edits;
      },
    }),
    vscode.languages.registerHoverProvider({ language: "plaintext" }, {
      provideHover(doc, pos) {
        const range = doc.getWordRangeAtPosition(pos);
        return range ? new vscode.Hover(new vscode.MarkdownString(`**${doc.getText(range)}**`), range) : undefined;
      },
    }),
  );
  vscode.commands.executeCommand("setContext", "sink.active", true);
}

function deactivate() {}

module.exports = { activate, deactivate };
