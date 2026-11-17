// Port of `crates/theme_importer/src/vscode/syntax.rs`: which TextMate scopes feed each Zed
// syntax token, how candidates are ranked, and the fallback chain. Keep the tables in sync with
// that file; any deliberate difference is listed in README.md.

export interface VsCodeTokenColor {
  name?: string;
  scope?: string | string[];
  settings: { foreground?: string; background?: string; fontStyle?: string };
}

/** Zed syntax token name → TextMate scopes, most specific first (`ZedSyntaxToken::to_vscode`). */
export const ZED_SYNTAX_TOKENS: ReadonlyArray<readonly [string, readonly string[]]> = [
  ["attribute", ["entity.other.attribute-name"]],
  ["boolean", ["constant.language"]],
  ["comment", ["comment"]],
  ["comment.doc", ["comment.block.documentation"]],
  ["constant", ["constant", "constant.language", "constant.character"]],
  ["constructor", ["entity.name.tag", "entity.name.function.definition.special.constructor"]],
  ["embedded", ["meta.embedded"]],
  ["emphasis", ["markup.italic"]],
  ["emphasis.strong", ["markup.bold", "markup.italic markup.bold", "markup.bold markup.italic"]],
  ["enum", ["support.type.enum"]],
  ["function", ["entity.function", "entity.name.function", "variable.function"]],
  ["hint", []],
  [
    "keyword",
    [
      "keyword",
      "keyword.other.fn.rust",
      "keyword.control",
      "keyword.control.fun",
      "keyword.control.class",
      "punctuation.accessor",
      "entity.name.tag",
    ],
  ],
  ["label", ["label", "entity.name", "entity.name.import", "entity.name.package"]],
  ["link_text", ["markup.underline.link", "string.other.link"]],
  ["link_uri", ["markup.underline.link", "string.other.link"]],
  ["number", ["constant.numeric", "number"]],
  ["operator", ["operator", "keyword.operator"]],
  ["predictive", []],
  ["preproc", ["preproc", "meta.preprocessor", "punctuation.definition.preprocessor"]],
  ["primary", []],
  [
    "property",
    ["variable.member", "support.type.property-name", "variable.object.property", "variable.other.field"],
  ],
  [
    "punctuation",
    [
      "punctuation",
      "punctuation.section",
      "punctuation.accessor",
      "punctuation.separator",
      "punctuation.definition.tag",
    ],
  ],
  [
    "punctuation.bracket",
    ["punctuation.bracket", "punctuation.definition.tag.begin", "punctuation.definition.tag.end"],
  ],
  ["punctuation.delimiter", ["punctuation.delimiter", "punctuation.separator", "punctuation.terminator"]],
  ["punctuation.list_marker", ["markup.list punctuation.definition.list.begin"]],
  ["punctuation.special", ["punctuation.special"]],
  ["string", ["string"]],
  ["string.escape", ["string.escape", "constant.character", "constant.other"]],
  ["string.regex", ["string.regex"]],
  ["string.special", ["string.special", "constant.other.symbol"]],
  ["string.special.symbol", ["string.special.symbol", "constant.other.symbol"]],
  ["tag", ["tag", "entity.name.tag", "meta.tag.sgml"]],
  ["text.literal", ["text.literal", "string"]],
  ["title", ["title", "entity.name"]],
  [
    "type",
    [
      "entity.name.type",
      "entity.name.type.primitive",
      "entity.name.type.numeric",
      "keyword.type",
      "support.type",
      "support.type.primitive",
      "support.class",
    ],
  ],
  [
    "variable",
    ["variable", "variable.language", "variable.member", "variable.parameter", "variable.parameter.function-call"],
  ],
  ["variable.special", ["variable.special", "variable.member", "variable.annotation", "variable.language"]],
  ["variant", ["variant"]],
];

/** `ZedSyntaxToken::fallbacks`. */
export const SYNTAX_FALLBACKS: Readonly<Record<string, readonly string[]>> = {
  "comment.doc": ["comment"],
  number: ["constant"],
  "variable.special": ["variable"],
  "punctuation.bracket": ["punctuation"],
  "punctuation.delimiter": ["punctuation"],
  "punctuation.list_marker": ["punctuation"],
  "punctuation.special": ["punctuation"],
  "string.escape": ["string"],
  "string.regex": ["string"],
  "string.special": ["string"],
  "string.special.symbol": ["string"],
};

const SCOPES = new Map(ZED_SYNTAX_TOKENS);

/** `ZedSyntaxToken::rank_match`: earlier scopes weigh more; exact scope string equality only. */
export function rankMatch(token: string, tokenColor: VsCodeTokenColor): number | undefined {
  if (tokenColor.scope === undefined) return undefined;
  const candidates = (Array.isArray(tokenColor.scope) ? tokenColor.scope : [tokenColor.scope]).flatMap(
    (scope) => scope.split(",").map((s) => s.trim()),
  );
  const toMatch = SCOPES.get(token) ?? [];
  let matches = 0;
  toMatch.forEach((scope, ix) => {
    const weight = toMatch.length - ix;
    if (candidates.includes(scope)) matches += 1 + weight;
  });
  return matches;
}

/**
 * `ZedSyntaxToken::find_best_token_color_match`. Rust's `Iterator::max_by_key` returns the last
 * of several equal maxima, so on a tie the later rule wins, which is also VS Code's precedence.
 */
export function findBestTokenColorMatch(
  token: string,
  tokenColors: readonly VsCodeTokenColor[],
): VsCodeTokenColor | undefined {
  let best: VsCodeTokenColor | undefined;
  let bestRank = 0;
  for (const tokenColor of tokenColors) {
    if (tokenColor.settings?.foreground === undefined) continue;
    const rank = rankMatch(token, tokenColor);
    if (rank === undefined || rank <= 0) continue;
    if (rank >= bestRank) {
      best = tokenColor;
      bestRank = rank;
    }
  }
  return best;
}
