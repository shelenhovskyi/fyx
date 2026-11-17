import { test } from "node:test";
import assert from "node:assert/strict";
import { appearanceOf, convertTheme, normalizeColor } from "../src/theme.ts";
import { findBestTokenColorMatch, rankMatch } from "../src/syntax.ts";

const theme = {
  name: "Fixture Dark",
  type: "dark",
  colors: {
    "editor.background": "#1e1e1e",
    "editor.foreground": "#D4D4D4",
    "panel.border": "#80808059",
    "tab.inactiveBackground": "#2d2d2d",
    "terminal.ansiBrightRed": "#f14c4c",
    "editorError.foreground": "#f00",
    "button.background": "not-a-colour",
  },
  tokenColors: [
    { settings: { foreground: "#cccccc" } },
    { scope: "comment", settings: { foreground: "#6a9955", fontStyle: "italic" } },
    { scope: ["keyword", "keyword.control"], settings: { foreground: "#569cd6", fontStyle: "bold" } },
    { scope: "string, string.quoted", settings: { foreground: "#ce9178" } },
    { scope: "constant", settings: { foreground: "#4fc1ff" } },
    { scope: "keyword", settings: { foreground: "#c586c0" } },
  ],
};

test("maps workbench colours like theme_importer", () => {
  const out = convertTheme(theme);
  const s = out.style;
  assert.equal(out.name, "Fixture Dark");
  assert.equal(out.appearance, "dark");
  assert.equal(s["background"], "#1e1e1eff");
  assert.equal(s["editor.gutter.background"], "#1e1e1eff");
  assert.equal(s["scrollbar.track.background"], "#1e1e1eff");
  assert.equal(s["toolbar.background"], "#1e1e1eff", "breadcrumb.background falls back to editor.background");
  assert.equal(s["editor.foreground"], "#d4d4d4ff");
  assert.equal(s["editor.active_line_number"], "#d4d4d4ff");
  assert.equal(s["border"], "#80808059");
  assert.equal(s["editor.wrap_guide"], "#80808059");
  assert.equal(s["tab.active_background"], "#2d2d2dff", "tab.activeBackground falls back to inactive");
  assert.equal(s["terminal.ansi.bright_red"], "#f14c4cff");
  assert.equal(s["text"], "#ccccccff", "text falls back to the unscoped token foreground");
  assert.equal(s["error"], "#ff0000ff");
  assert.equal(s["hint"], "#969696ff", "hint has the importer's default");
  assert.equal(s["element.background"], undefined, "invalid colours are dropped");
  assert.equal(s["background.appearance"], "opaque");
});

test("maps syntax tokens with ranking, ties and fallbacks", () => {
  const syntax = convertTheme(theme).style.syntax as Record<string, Record<string, unknown>>;
  assert.deepEqual(syntax.comment, { color: "#6a9955ff", font_style: "italic" });
  assert.deepEqual(syntax["comment.doc"], syntax.comment, "comment.doc falls back to comment");
  // Both keyword rules match "keyword"; the array rule also matches "keyword.control" and ranks higher.
  assert.deepEqual(syntax.keyword, { color: "#569cd6ff", font_weight: 700 });
  assert.equal(syntax.string.color, "#ce9178ff", "comma-separated scopes are split");
  assert.equal(syntax["string.regex"].color, "#ce9178ff", "string.regex falls back to string");
  assert.equal(syntax.number.color, "#4fc1ffff", "number falls back to constant");
  assert.equal(syntax.type, undefined);
});

test("equal ranks resolve to the later rule, like Rust's max_by_key", () => {
  const rules = [
    { scope: "keyword", settings: { foreground: "#111111" } },
    { scope: "keyword", settings: { foreground: "#222222" } },
  ];
  assert.equal(rankMatch("keyword", rules[0]), 1 + 7);
  assert.equal(findBestTokenColorMatch("keyword", rules)?.settings.foreground, "#222222");
});

test("appearance comes from uiTheme or type", () => {
  assert.equal(appearanceOf({}, "vs"), "light");
  assert.equal(appearanceOf({}, "hc-light"), "light");
  assert.equal(appearanceOf({}, "vs-dark"), "dark");
  assert.equal(appearanceOf({ type: "light" }), "light");
  assert.equal(appearanceOf({}), "dark");
});

test("normalizes hex colours", () => {
  assert.equal(normalizeColor("#ABC"), "#aabbccff");
  assert.equal(normalizeColor("#abcd"), "#aabbccdd");
  assert.equal(normalizeColor("#a1b2c3"), "#a1b2c3ff");
  assert.equal(normalizeColor("#a1b2c3d4"), "#a1b2c3d4");
  assert.equal(normalizeColor("rgba(0,0,0,1)"), undefined);
  assert.equal(normalizeColor("#12345"), undefined);
});
