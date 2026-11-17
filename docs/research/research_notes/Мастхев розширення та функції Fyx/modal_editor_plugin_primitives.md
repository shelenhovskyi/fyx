# Must-have plugins in Neovim, Emacs, Helix, Sublime Text and Lapce, and the UI primitives behind them (state: 17 November 2026)

Method note: GitHub star counts and last-push dates were pulled from the GitHub REST API (`gh api repos/<owner>/<repo>`) on 2026-11-17. MELPA numbers are **cumulative all-time downloads** from https://melpa.org/download_counts.json, fetched 2026-11-17. That favors old packages and undercounts packages that are now built into Emacs. Package Control numbers are all-time installs from https://packagecontrol.io/browse/popular, fetched 2026-11-17. The "primitive used" column is mostly analyst knowledge of each plugin's architecture, checked against the official API docs cited below. It was not re-read from each README this session, so treat it as inference unless a citation says otherwise.

## Q1. Top Neovim plugins: popularity, function, primitives

### Takeaway
The most popular Neovim plugins share a small set of primitives: fuzzy pickers in floating windows, extmarks (signs, virtual text, virtual lines, highlights, conceal) and "buffers as UI" (fugitive, neogit, oil, octo). Pickers (telescope ~19.9k stars) and AI sidebars (avante ~18.2k) top the UI-plugin star counts. Plugin managers and treesitter top install counts.

### Cited Findings
**Popularity (GitHub stars, 2026-11-17, with last push)**
- Core and infra, for scale: neovim/neovim 103,954 — [GitHub](https://github.com/neovim/neovim); folke/lazy.nvim 21,734 (last push 2026-06-29) — [GitHub](https://github.com/folke/lazy.nvim); nvim-treesitter 14,544 — [GitHub](https://github.com/nvim-treesitter/nvim-treesitter); mason.nvim 10,537 — [GitHub](https://github.com/mason-org/mason.nvim); nvim-cmp 9,489 — [GitHub](https://github.com/hrsh7th/nvim-cmp); blink.cmp 6,674 — [GitHub](https://github.com/Saghen/blink.cmp)
- Pickers: telescope.nvim 19,861 (pushed 2026-11-17) — [GitHub](https://github.com/nvim-telescope/telescope.nvim); snacks.nvim (includes snacks.picker) 8,224 (last push 2026-05-25) — [GitHub](https://github.com/folke/snacks.nvim); fzf-lua 4,496 — [GitHub](https://github.com/ibhagwan/fzf-lua)
- Collection: mini.nvim 9,669 (repo moved from echasnovski/ to the nvim-mini/ org) — [GitHub](https://github.com/nvim-mini/mini.nvim)
- Navigation: harpoon 9,244 (last push 2025-10-31) — [GitHub](https://github.com/ThePrimeagen/harpoon); oil.nvim 6,984 — [GitHub](https://github.com/stevearc/oil.nvim); leap.nvim 5,028 — [GitHub](https://github.com/ggandor/leap.nvim); flash.nvim 4,280 — [GitHub](https://github.com/folke/flash.nvim); nvim-tree.lua 8,691 — [GitHub](https://github.com/nvim-tree/nvim-tree.lua); neo-tree.nvim 5,656 — [GitHub](https://github.com/nvim-neo-tree/neo-tree.nvim); aerial.nvim 2,360 — [GitHub](https://github.com/stevearc/aerial.nvim)
- Git: vim-fugitive 21,828 — [GitHub](https://github.com/tpope/vim-fugitive); gitsigns.nvim 7,188 — [GitHub](https://github.com/lewis6991/gitsigns.nvim); diffview.nvim 5,889 (**last push 2024-08-02**, which looks stale) — [GitHub](https://github.com/sindrets/diffview.nvim); neogit 5,678 — [GitHub](https://github.com/NeogitOrg/neogit); octo.nvim 3,410 — [GitHub](https://github.com/pwntester/octo.nvim); lazygit.nvim 2,384 (wraps lazygit, 83,945 stars) — [GitHub](https://github.com/kdheepak/lazygit.nvim), [lazygit](https://github.com/jesseduffield/lazygit)
- UI chrome: lualine.nvim 8,188 — [GitHub](https://github.com/nvim-lualine/lualine.nvim); which-key.nvim 7,341 (last push 2025-10-28) — [GitHub](https://github.com/folke/which-key.nvim); noice.nvim 5,832 (last push 2025-11-03) — [GitHub](https://github.com/folke/noice.nvim); indent-blankline 5,008 — [GitHub](https://github.com/lukas-reineke/indent-blankline.nvim); nvim-notify 3,571 — [GitHub](https://github.com/rcarriga/nvim-notify); zen-mode 2,166 — [GitHub](https://github.com/folke/zen-mode.nvim)
- Diagnostics and lists: trouble.nvim 6,915 — [GitHub](https://github.com/folke/trouble.nvim); todo-comments.nvim 4,229 — [GitHub](https://github.com/folke/todo-comments.nvim)
- Tooling: conform.nvim 5,403 — [GitHub](https://github.com/stevearc/conform.nvim); nvim-lint 2,787 — [GitHub](https://github.com/mfussenegger/nvim-lint); toggleterm 5,622 (last push 2025-03-09) — [GitHub](https://github.com/akinsho/toggleterm.nvim); overseer.nvim 1,954 — [GitHub](https://github.com/stevearc/overseer.nvim); neotest 3,122 — [GitHub](https://github.com/nvim-neotest/neotest); nvim-dap-ui 3,389 — [GitHub](https://github.com/rcarriga/nvim-dap-ui)
- Editing: nvim-surround 4,326 — [GitHub](https://github.com/kylechui/nvim-surround)
- Notes and markdown: render-markdown.nvim 5,256 — [GitHub](https://github.com/MeanderingProgrammer/render-markdown.nvim); epwalsh/obsidian.nvim 6,225 (original), with active community fork obsidian-nvim/obsidian.nvim 2,250 (pushed 2026-11-17) — [original](https://github.com/epwalsh/obsidian.nvim), [fork](https://github.com/obsidian-nvim/obsidian.nvim); nvim-orgmode 3,945 — [GitHub](https://github.com/nvim-orgmode/orgmode)
- AI: avante.nvim 18,198 (repo moved to avante-corp/) — [GitHub](https://github.com/yetone/avante.nvim); codecompanion.nvim 6,945 — [GitHub](https://github.com/olimorris/codecompanion.nvim)

**Install-based ranking (Dotfyle)**
- Dotfyle ranks plugins by installs across "1000+" tracked configs, with 100,784 plugin installations in total. Counts it lists: nvim-treesitter 2,539 configs, nvim-lspconfig 2,396, plenary.nvim 2,314, lazy.nvim 2,282. The other number Dotfyle shows next to each plugin (for example 21,708 for lazy.nvim) matches GitHub stars, not installs — [Dotfyle top](https://www.dotfyle.com/neovim/plugins/top). An older search-engine snippet of the same page showed "lazy.nvim 18190", which is probably a stale star count — [Dotfyle top (search snippet)](https://www.dotfyle.com/neovim/plugins/top). The full top-60 list did not render in the fetch.

**Primitives in the Neovim API that these plugins build on (official docs)**
- Extmarks are buffer annotations that track logical positions as text changes. They work "like a 'vertical bar' cursor", have gravity, survive deletion of surrounding text and are restored on undo/redo — [Neovim API docs](https://neovim.io/doc/user/api/)
- One `nvim_buf_set_extmark` call covers highlights (`hl_group`, `line_hl_group`, `number_hl_group`, `cursorline_hl_group`), signs (`sign_text`, `sign_hl_group`), virtual text (`virt_text` with `virt_text_pos` = `eol`, `eol_right_align`, `overlay`, `right_align`, `inline`), virtual lines (`virt_lines`, `virt_lines_above`, `virt_lines_leftcol`), concealing (`conceal`, `conceal_lines`), plus `priority`, `ephemeral`, `url`, `invalidate` and `undo_restore` — [Neovim API docs](https://neovim.io/doc/user/api/)
- Namespaces (`nvim_create_namespace`) let each plugin manage and clear only its own marks and highlights — [Neovim API docs](https://neovim.io/doc/user/api/)
- Decoration providers (`nvim_set_decoration_provider`) are called during redraw: `on_start`, `on_buf`, `on_win`, `on_range` and `on_end`. `on_line` is now marked "deprecated, use on_range instead". This lets plugins decorate only the visible viewport, using ephemeral marks — [Neovim API docs](https://neovim.io/doc/user/api/)
- Floating windows (`nvim_open_win`): `relative` = `cursor`, `editor`, `laststatus`, `mouse`, `tabline` or `win`; `anchor` = NW, NE, SW or SE; `border`; `title` and `footer` with positions; `focusable`, `hide`, `fixed`, `mouse`; and `zindex` (default 50, "Below 100 is recommended"). Floats do not support scrollbars — [Neovim API docs](https://neovim.io/doc/user/api/)
- Scratch buffers (`nvim_create_buf(listed, scratch)`) are "throwaway" buffers, always 'nomodified'. They are the substrate for buffers-as-UI — [Neovim API docs](https://neovim.io/doc/user/api/)
- API contract: functions carry `since` and `deprecated_since`. "New functions MAY CHANGE before release." After release, signatures change only through approved extensions such as an added `opts` parameter. "Deprecated functions will not be removed until Nvim 2.0". `nvim__` functions and undocumented items are not covered — [Neovim API docs](https://neovim.io/doc/user/api/)

### Inferences
- Primitive mapping (analyst knowledge, not re-verified per README):
  - **telescope, fzf-lua, snacks.picker, mini.pick:** floating windows, a prompt buffer, async sources, a previewer window, actions on the selection.
  - **harpoon:** a floating editable buffer as its menu, plus persistent per-project state.
  - **oil:** buffer-as-UI. The directory is an editable buffer and saving applies a filesystem diff. It uses a custom `oil://` URI scheme and conceal for IDs.
  - **gitsigns:** extmark signs, line and number highlights, EOL virt_text (blame), virt_lines or a float for inline hunk preview, and hunk text objects.
  - **fugitive, neogit:** buffers-as-UI (status buffer with sections and folds, per-line keymaps) and virtual documents (`fugitive://`).
  - **diffview:** a tabpage layout with diff-mode windows and a file-panel tree buffer.
  - **trouble:** a split list buffer with extmarks and a tree.
  - **which-key:** keymap introspection, a pending-key timeout hook and a floating window.
  - **flash, leap:** extmark `overlay` virt_text labels, a modal key-capture loop (getchar) and multi-window search.
  - **lualine:** statusline, winbar and tabline segments.
  - **noice:** `vim.ui_attach` external messages and cmdline (replaces core UI), floats and notifications.
  - **todo-comments:** highlights, signs and picker integration.
  - **neotest:** signs, virt_text results, a float for output, a tree summary panel and async jobs.
  - **dap-ui:** split-window layouts of tree buffers and hover floats.
  - **conform:** save hooks and minimal-diff text edits.
  - **overseer:** async jobs and terminals, a task list buffer and status-line segments.
  - **octo:** buffers-as-UI for issues and PRs (`octo://` editable buffers), virt_text and signs for review threads, a picker.
  - **render-markdown:** conceal, inline and overlay virt_text, virt_lines and a decoration provider.
  - **avante, codecompanion:** sidebar split chat buffers, streaming, and inline diff via extmarks and virt_lines.
- Neovim's key design move is that **one anchor primitive (the extmark) carries every decoration kind, owned by a namespace**, plus a viewport-scoped redraw callback for performance. Fyx's CRDT-anchored decorations are the analogue. Signs, EOL and inline virtual text, line highlights and conceal are the gaps relative to block decorations alone.

### Gaps
- Full Dotfyle top-60 with install counts did not render. Only four trending entries had config counts.
- No direct 2026 awesome-neovim ranking was fetched. awesome-neovim is a categorized list with no ranking.

## Q2. Top Emacs packages, and why magit's "buffers as UI" model is loved

### Takeaway
On MELPA, magit (5.37M all-time downloads) is the most-downloaded end-user package; only the utility libraries dash and s rank higher. The modern completion stack (vertico, consult, embark, orderless, marginalia) is a set of small packages composing over one shared protocol, `completing-read`. Magit's appeal comes from typed, collapsible sections in ordinary text buffers, where every key acts on the "thing at point", plus transient prefix menus for discoverability.

### Cited Findings
- MELPA all-time downloads, fetched 2026-11-17 — [MELPA download_counts.json](https://melpa.org/download_counts.json):
  - **Git:** magit 5,374,279; transient 3,298,599; magit-section 2,053,007; forge 810,725; diff-hl 894,991; git-gutter 489,995
  - **Navigation and projects:** projectile 3,369,449; avy 2,557,385; ace-window 2,093,908; treemacs 1,761,958
  - **Key help and modal:** which-key 2,197,152; hydra 2,182,800; evil 2,141,976; evil-collection 850,714
  - **IDE:** lsp-mode 2,612,917; lsp-ui 1,128,278; dap-mode 831,622; eglot 146,609 (built into Emacs, so MELPA undercounts it)
  - **Completion, old stack:** helm 3,919,058; ivy 1,923,236; counsel 1,305,417; company 3,704,503
  - **Completion, modern stack:** consult 779,476; marginalia 549,469; orderless 482,432; vertico 473,247; embark 364,749; corfu 302,164
  - **Child frames:** posframe 1,349,373
  - **Notes and UI:** org-roam 507,205; doom-modeline 1,080,339
  - **Terminal:** vterm 610,627
  - **AI:** gptel 269,685; ellama 50,194
- The top of the MELPA all-time list is dominated by libraries (dash 6.58M, s 5.70M, f 4.52M), then magit 5.37M, helm, company, markdown-mode, flycheck, projectile and transient — [MELPA download_counts.json](https://melpa.org/download_counts.json)
- GitHub stars (2026-11-17):
  - **Distributions:** spacemacs 24,561 — [GitHub](https://github.com/syl20bnr/spacemacs); doomemacs 22,829 — [GitHub](https://github.com/doomemacs/doomemacs)
  - **Git:** magit 7,259 — [GitHub](https://github.com/magit/magit); forge 1,586 — [GitHub](https://github.com/magit/forge)
  - **IDE:** lsp-mode 5,138 — [GitHub](https://github.com/emacs-lsp/lsp-mode); eglot 2,574 — [GitHub](https://github.com/joaotavora/eglot)
  - **Navigation and modal:** projectile 4,250 — [GitHub](https://github.com/bbatsov/projectile); evil 3,872 — [GitHub](https://github.com/emacs-evil/evil); avy 1,967 (last push 2024-11-01) — [GitHub](https://github.com/abo-abo/avy)
  - **AI:** gptel 3,573 — [GitHub](https://github.com/karthink/gptel)
  - **Completion:** vertico 1,897 — [GitHub](https://github.com/minad/vertico); consult 1,680 — [GitHub](https://github.com/minad/consult); corfu 1,556 — [GitHub](https://github.com/minad/corfu); embark 1,235 — [GitHub](https://github.com/oantolin/embark); orderless 1,058 — [GitHub](https://github.com/oantolin/orderless); marginalia 997 — [GitHub](https://github.com/minad/marginalia)
- The standalone emacs-which-key repo is archived (archived=true, last push 2024-06-25), consistent with which-key having moved into Emacs core (Emacs 30) — [GitHub](https://github.com/justbur/emacs-which-key)
- Magit buffers are built from nested, collapsible sections. Each section has a type (`hunk`, `file`, `commit`…) and optionally a value, and each type can have its own keymap (`magit-TYPE-section-map`). Many commands act on the section at point. With an active region, they apply to all selected sections of the same type — [Magit manual: Sections](https://docs.magit.vc/magit/Sections.html)
- Section navigation (`n`/`p`/`M-n`/`M-p`/`^`), visibility cycling (TAB, `C-<tab>`, `S-<tab>`, levels 1–4), and refresh that preserves visibility. Movement hooks can update a companion window (revision or blob buffer) to follow point — [Magit manual: Sections](https://docs.magit.vc/magit/Sections.html)
- Magit's command menus are "transient" commands, successors of magit-popup and magit-key-mode — [Magit manual: Transient Commands](https://magit.vc/manual/magit/Transient-Commands.html). Transient is named for "the temporary keymap and the popup UI for displaying that keymap", and supports nested sub-menus that build up a command like a phrase — [transient-showcase](https://github.com/positron-solutions/transient-showcase)
- The transient FAQ records a deliberate key decision: whether `q` or `C-g` quits, so that `q` stays bindable inside menus — [Transient FAQ](https://docs.magit.vc/transient/FAQ.html)
- A critique on emacs-devel argues that magit exposes git commands 1-to-1 as pick-from-list menus. The defense is that transient makes git's complexity manageable — [emacs-devel thread](https://yhetil.org/emacs/p91h7g8yisj.fsf@google.com/), [reply](https://yhetil.org/emacs/AM9PR09MB4977E3F259D739FD74FB666296F29@AM9PR09MB4977.eurprd09.prod.outlook.com/)

### Inferences
- Why buffers-as-UI wins for keyboard users: UI rendered as text in a normal buffer inherits search, motions, yank, macros, the region, narrowing and theming for free. Typed sections give stable "what is at point" semantics for context actions. Magit-section is reused by other packages: its 2.05M downloads, separate from magit, imply this.
- The vertico/consult/embark/orderless/marginalia stack shows a **protocol-first picker**. Each package plugs into a different stage of the same pipeline:
  - vertico: the UI
  - orderless: matching
  - marginalia: annotations
  - consult: sources
  - embark: actions on candidates
- That composability is what Fyx's planned quick-pick should copy: sources, matchers, annotators and actions as separately pluggable stages, not a monolithic picker per extension.
- Emacs's primitive set behind its top packages: buffers, text properties and overlays (before-string and after-string, the Emacs version of virtual text), fringe bitmaps (signs), mode-line and header-line, minibuffer and `completing-read`, transient keymaps (avy, hydra, transient), child frames (posframe: 1.35M downloads, so floating popups are in demand even in Emacs), and process filters for async (gptel streaming, magit git calls).

### Gaps
- MELPA has no recent or windowed download stats, only all-time counts. Recent momentum (for example gptel vs. ellama) cannot be measured from it.
- Org-mode and eglot are built into Emacs, so I found no usage numbers for them.
- I found no single authoritative essay on "why magit is loved". The evidence above comes from the manual and the transient docs.

## Q3. Helix, Lapce, Sublime Text and Kakoune: plugin-system status and primitives

### Takeaway
Helix still has no merged plugin system in November 2026: the Steel (Scheme) PR #8675 is an open draft after about three years, and a community ecosystem has grown on the fork. Lapce's WASI "volts" are mostly LSP and capability wrappers with little UI surface. Sublime offers the richest classic GUI plugin primitives: phantoms, popups in minihtml, regions and gutter icons, quick panel, command-palette input handlers and HTML sheets. Kakoune deliberately has no plugin language: it uses `%sh{}` and a client-server session protocol.

### Cited Findings
**Helix**
- PR "Add Steel as an optional plugin system" (#8675): state=open, draft=true, merged=false, 336 comments, last updated 2026-11-17 — [GitHub PR #8675](https://github.com/helix-editor/helix/pull/8675) (via GitHub API)
- PR comments, Aug–Sep 2026:
  - One user said they had been "daily driving this fork for a while with no major issues" — [PR #8675](https://github.com/helix-editor/helix/pull/8675)
  - Another worried that an ecosystem built around a draft PR "creates false hopes and expectations"; a third wrote "I'm becoming less optimistic about it landing soon" — [PR #8675](https://github.com/helix-editor/helix/pull/8675)
- Plugin projects in 2026 tell users to build the `steel-event-system` branch of the fork. NixOS packages a "steelix" fork with Steel scripting, versioned 2026-05-21 — [helix-plugins-nix](https://codeberg.org/nx2/helix-plugins-nix), [mynixos steelix](https://mynixos.com/nixpkgs/package/steelix). An Arch wiki mirror (2026-04) states the plugin system has not been merged into mainline — [Arch wiki mirror](https://wikipedia.jakami.de/content/archlinux_en_all_maxi_2026-04/Helix)
- Latest Helix releases per the GitHub API: 25.07.1 (2025-07-18) and 25.07 (2025-07-15). There has been no newer tagged release. helix 46,915 stars; steel 2,662 stars — [Helix releases](https://github.com/helix-editor/helix/releases), [steel](https://github.com/mattwparas/steel)
- Earlier user threads on plugin status and on using the plugins branch — [r/HelixEditor "plugins state" (Mar 2025)](https://axebps-redlib.hf.space/r/HelixEditor/comments/1jiy1r9/plugins_state), [r/HelixEditor plugins branch thread](https://redlib.hbubli.cc/r/HelixEditor/comments/1l11g2y/anyone_already_using_the_plugins_branch_in_your)

**Lapce**
- Plugins ("volts") are WASI modules written in any language that compiles to WASI. A `volt.toml` manifest declares activation (language and workspace-contains regexes) and settings that appear in Lapce's UI. Outbound HTTP goes through `lapce_wasi_experimental_http` — [Lapce plugin docs](https://docs.lapce.dev/development/plugin-development), [lapce-plugin crate](https://docs.rs/crate/lapce-plugin/0.1.2/source/src/lib.rs), [blog: how to write a Lapce LSP plugin](https://oknozor.github.io/blog/how-to-lapce-plugin)
- The Plugin Server Protocol (PSP) negotiates capabilities at handshake, such as whether a plugin may start LSP servers or make HTTP requests (default false) — [PSP spec](https://lapce.dev/plugin-server-protocol/specifications/psp/0.1/specification/)
- The `lapce-plugin` crate is at 0.1.2 with low documentation coverage — [docs.rs](https://docs.rs/crate/lapce-plugin/latest)
- lapce 39,084 stars; last stable v0.4.6 (2026-01-21); nightly 2026-10-09; last push to the repo 2026-10-09 — [GitHub](https://github.com/lapce/lapce)

**Sublime Text**
- Phantoms (`PhantomSet`/`Phantom`) with layouts `INLINE`, `BELOW` and `BLOCK`. Popups (`View.show_popup` with `PopupFlags`) and HTML sheets (`Window.new_html_sheet`) use minihtml — [Sublime API reference](https://www.sublimetext.com/docs/api_reference.html)
- Other primitives — [Sublime API reference](https://www.sublimetext.com/docs/api_reference.html):
  - `View.add_regions` with `RegionFlags` (solid, stippled or squiggly underlines, gutter icons)
  - `Window.show_quick_panel` with `on_select` (-1 on cancel), `on_highlight` (live preview), a placeholder and `QuickPanelFlags` (`KEEP_OPEN_ON_FOCUS_LOST`, `WANT_EVENT`, monospace)
  - Command-palette input handlers (`ListInputHandler`, `TextInputHandler`, `BackInputHandler`)
  - Output panels and I/O panels with an input box
  - `status_message`
  - `EventListener`, `ViewEventListener`, `TextChangeListener`, `on_hover` with `HoverZone` (text, gutter…), and completions via `CompletionList`
- Package Control all-time installs: Emmet 6.15M, SideBarEnhancements 2.95M, BracketHighlighter 2.67M, SublimeLinter 2.34M, AutoFileName 1.63M, Alignment 1.51M, SublimeREPL 1.48M, A File Icon 1.35M, SFTP 1.28M, Color Highlighter 1.27M, Git 1.23M, DocBlockr 1.21M — [Package Control popular](https://packagecontrol.io/browse/popular). sublimelsp/LSP 1,813 stars, pushed 2026-11-17 — [GitHub](https://github.com/sublimelsp/LSP)

**Kakoune**
- The scripting language is meant as "a glue between core Kakoune instructions and complex logic". Complex work is delegated to external programs through `%sh{}` shell scopes, with editor state passed in environment variables. Custom interpreter scopes were rejected because they "would add hard dependencies" — [Kakoune FAQ](https://github.com/mawww/kakoune/blob/master/doc/pages/faq.asciidoc)
- External tools drive a running session by piping commands to `kak -p $kak_session` (for example `eval -client $kak_client edit /path`). Window splitting is left to tmux, following the UNIX philosophy — [Kakoune FAQ](https://github.com/mawww/kakoune/blob/master/doc/pages/faq.asciidoc). kakoune 11,146 stars — [GitHub](https://github.com/mawww/kakoune)

### Inferences
- Helix shows the cost of not shipping any extension API. Users either fork (steelix), wait three years, or leave. Zed closed its "Helix keymap" request (see Q4), which absorbs some of that demand: Fyx can target Helix refugees who want plugins.
- Lapce shows that a sandboxed WASM plugin system with **no UI primitives** yields an ecosystem of LSP wrappers. UI surface, not the sandbox, drives plugin variety. This supports Fyx's `fyx:ui` direction.
- Sublime's minihtml phantoms and popups resemble Fyx's "theme-tokens-only retained tree": a restricted markup avoids full-webview cost while still allowing rich inline UI. Its quick panel's `on_highlight` (preview on move) and input handlers chaining inside the command palette are worth copying into Fyx's quick-pick.
- Kakoune's client-server model suggests Fyx expose a CLI/IPC "send command to session" path. Many keyboard-centric workflows (lazygit, yazi, fzf, broot) integrate this way without a plugin.

### Gaps
- I found no official list of Lapce plugin UI extension points. Lapce appears to have none beyond settings, but this is unconfirmed.
- The Sublime plugin_host Python versions (3.3 and 3.8, and whether newer builds added more) were not visible in the fetched part of the API page.
- I did not check whether the Steel API surface in the fork includes UI components (pickers, popups) beyond commands and keymaps.

## Q4. What Neovim, Helix and JetBrains users miss in Zed, and Zed's native coverage

### Takeaway
Zed has absorbed many top plugins natively: surround, sneak-like motions, which-key (shipped), rainbow brackets (Dec 2025), the Helix keymap (Dec 2025), diagnostics, debugger, git gutter and panel. The highest-voted gaps are UI-extensibility gaps: webviews and custom UI from extensions, flash-style labeled jumps, side-by-side git diff, PR review (octo/forge-like), commit log and local history. Harpoon-, oil-, octo-, obsidian- and todo-comments-style plugins have no native equivalent.

### Cited Findings
- Zed GitHub issue reactions, via API on 2026-11-17:
  - "Webview via Extensions" #21208: open, 477 👍 / 506 total — [GitHub](https://github.com/zed-industries/zed/issues/21208)
  - "Flash.nvim style search in a document" #14801: open, 401 👍 / 488 total, 77 comments — [GitHub](https://github.com/zed-industries/zed/issues/14801)
  - "Helix keymap" #4642: closed as completed 2025-12-19, 1,098 reactions — [GitHub](https://github.com/zed-industries/zed/issues/4642)
  - "Rainbow brackets" #5259: closed as completed 2025-12-03, 1,270 reactions — [GitHub](https://github.com/zed-industries/zed/issues/5259)
  - "Add a plugin interface to extend the editor" #5269: closed as completed 2024-04-18, 714 reactions — [GitHub](https://github.com/zed-industries/zed/issues/5269)
  - "WYSIWYG Markdown Preview" #21717: open — [GitHub](https://github.com/zed-industries/zed/issues/21717)
- Zed discussions by upvotes (2026-11-17):
  - "Side-by-side Git View" 927 — [#26770](https://github.com/zed-industries/zed/discussions/26770)
  - "GitHub integration for reviewing PRs directly inside of Zed" 637 — [#34759](https://github.com/zed-industries/zed/discussions/34759)
  - "Settings Sync" 379 — [#6569](https://github.com/zed-industries/zed/discussions/6569)
  - "Jupyter Notebook" 317 — [#25936](https://github.com/zed-industries/zed/discussions/25936)
  - "Commit Log" 316 — [#26511](https://github.com/zed-industries/zed/discussions/26511)
  - "Local History for Files" 286 — [#24004](https://github.com/zed-industries/zed/discussions/24004)
  - "Are we Helix yet?" 124 — [#33580](https://github.com/zed-industries/zed/discussions/33580)
  - "Implement Leap Key Navigation" 28 — [#25470](https://github.com/zed-industries/zed/discussions/25470)
- Zed vim mode built-ins — [Zed docs: Vim](https://zed.dev/docs/vim):
  - surround (`ys`/`cs`/`ds`), exchange (`cx`), commentary (`gc`/`gcc`), ReplaceWithRegister (`gR`)
  - Multicursor (`g l`/`g L`/`g a`) and treesitter text objects (`af`/`if`, `ac`/`ic`, `aa`/`ia`, `at`/`it`)
  - Sneak via an opt-in binding (`vim::PushSneak`) and labeled word jumps via `vim::HelixJumpToWord` (no default binding)
  - Project search `g /`, symbol pickers `g s`/`g S`, and netrw-style project panel keys
- Zed docs on Vim's command line: "commands currently do not support arguments" and "We don't emulate the full power of Vim's command line yet". The page mentions no which-key or harpoon-like marks — [Zed docs: Vim](https://zed.dev/docs/vim)
- Which-key: the feature request #10910 is marked completed, and PR #43618 (Jan 2026) added the popup. A community guide shows a `which_key` setting with `delay_ms` — [issue #10910](https://github.com/zed-industries/zed/issues/10910), [PR #43618 mirror](https://lobakmerak.netlify.app/host-https-github.com/zed-industries/zed/pull/43618), [zed-101-setup wiki](https://github.com/jellydn/zed-101-setup/wiki). These came from search snippets, not the official docs.
- Zed extension types documented: languages, themes, icon themes, debuggers, MCP servers and snippets. There is no mention of custom UI (panels, pickers, decorations) — [Zed docs: Extensions](https://zed.dev/docs/extensions)
- Extension permissions are capability-based: `process:exec`, `download_file` (scopable by host and path) and `npm:install`, restricted via `granted_extension_capabilities`. No UI APIs are described — [Zed docs: Extension capabilities](https://zed.dev/docs/extensions/capabilities)
- Switcher posts:
  - Zed bundles git, LSP and fuzzy finding, so users drop plugin maintenance. Zed's comparison page says Neovim remains better for terminal/SSH work — [Zed vs Neovim (vendor page, updated 2026-05-01)](https://zed.dev/compare/neovim), [Leaving Neovim for Zed](https://unixism.net/2024/08/leaving-neovim-for-zed/), [Trying out Zed after a decade of Vim](https://hn.jakechampion.name/reader/https://sgoel.dev/posts/trying-out-zed-after-more-than-a-decade-of-vim-neovim)
  - Zed's team wrote earlier that registers and macros were missing from vim mode — [Zed blog: Zed decoded – Vim](https://zed.dev/blog/zed-decoded-vim)
  - One 2026-03-16 post went back from Zed to Neovim. The stated reason was trust after Zed's terms-of-service change (18+ age restriction), not missing features. Its new setup is mini.nvim-centric: mini.pick "replaces telescope", plus mini.files and mini.completion — [Returning to Neovim](https://stevedylandev.pages.dev/posts/returning-to-neovim)

### Inferences
- Zed native coverage per plugin (my assessment from Zed docs and issues; **verify the items not fetched this session**):
  - **telescope / fzf-lua / snacks.picker:** partial (file finder, project search, symbols; no user-defined picker sources).
  - **harpoon:** none.
  - **oil:** none (the project panel is not an editable buffer).
  - **gitsigns:** mostly (gutter hunks, inline blame).
  - **fugitive / neogit / magit:** partial (git panel; no commit log as of this snapshot, discussion #26511).
  - **diffview:** partial (project diff multibuffer; side-by-side requested, discussion #26770).
  - **trouble:** yes (diagnostics multibuffer).
  - **which-key:** yes (since #43618).
  - **flash / leap / avy:** partial (`HelixJumpToWord`, sneak; #14801 still open).
  - **lualine:** no customization.
  - **noice:** n/a (GUI).
  - **todo-comments:** none.
  - **neotest:** partial (runnables and tasks).
  - **dap-ui:** yes (built-in debugger).
  - **conform:** yes (formatter settings).
  - **overseer:** yes-ish (tasks).
  - **octo / forge:** none (discussion #34759).
  - **obsidian / org:** none.
  - **avante / codecompanion / gptel:** yes (agent panel).
  - **render-markdown:** partial (preview pane; WYSIWYG requested in #21717).
- The open, high-vote items (#21208 webview, #14801 flash, side-by-side diff, PR review, commit log, local history) are exactly what a `fyx:ui` API with panels, decorations, key capture and host diff components would unlock for extensions.

### Gaps
- I did not fetch the Zed docs pages for debugger, tasks, git panel and outline this session. The native-coverage verdicts above are from analyst knowledge plus the issue and discussion states.
- I found no systematic survey of "what Neovim users miss in Zed". The evidence is anecdotal blog posts plus GitHub votes.

## Q5. What breaks plugin ecosystems

### Takeaway
The observed failure modes are API churn and missing stability contracts (Helix's unmerged API; Neovim deprecations such as `on_line` → `on_range`), single-maintainer concentration and repo moves, shared single-threaded extension hosts (VS Code), and too-narrow APIs (Lapce, Zed's UI-less extensions).

### Cited Findings
- Neovim's contract: released signatures are frozen (except added `opts`), deprecated functions survive "until Nvim 2.0", and prerelease and `nvim__` APIs are explicitly unstable. Even so, decoration provider `on_line` is now deprecated in favor of `on_range` — [Neovim API docs](https://neovim.io/doc/user/api/)
- VS Code runs all extensions in one Node.js extension host process. That isolates the editor from extensions but not extensions from each other: one extension can monopolize the single thread and block all others. VS Code attaches a CPU profiler when the host goes unresponsive — [vscode-wiki: Explain extension causes high CPU load](https://github.com/pajenterprise/vscode-wiki/blob/master/Explain-extension-causes-high-cpu-load.md). Real-world reports cite hosts unresponsive for about 15 s to 2.5 min from one extension's synchronous work — [issue report](https://claudeissues.com/issue/69473-vs-code-extension-host-becomes-unresponsive-1-2-5-min-while-claude-exe-resume-lo), [issue report](https://claudeissues.com/issue/67948-vs-code-extension-synchronous-shell-launch-in-resolveshellpath-blocks-event-loop) (anecdotal user issue mirrors)
- Maintainer concentration and staleness (GitHub API, 2026-11-17):
  - Most folke plugins (which-key, trouble, noice, todo-comments, zen-mode) were last pushed Oct–Nov 2025; snacks.nvim on 2026-05-25 and lazy.nvim on 2026-06-29 — [which-key](https://github.com/folke/which-key.nvim), [trouble](https://github.com/folke/trouble.nvim), [noice](https://github.com/folke/noice.nvim), [snacks](https://github.com/folke/snacks.nvim), [lazy](https://github.com/folke/lazy.nvim)
  - diffview.nvim was last pushed 2024-08-02 — [diffview](https://github.com/sindrets/diffview.nvim)
  - obsidian.nvim continued as a community fork — [obsidian-nvim/obsidian.nvim](https://github.com/obsidian-nvim/obsidian.nvim)
  - mini.nvim and avante moved organizations — [mini.nvim](https://github.com/nvim-mini/mini.nvim), [avante](https://github.com/yetone/avante.nvim)
- Helix: about three years in draft, with community frustration in the PR thread, as cited in Q3 — [PR #8675](https://github.com/helix-editor/helix/pull/8675)
- Switchers cite the plugin-maintenance burden: updating plugins "inevitably breaks things", a reason to prefer batteries-included Zed — [Leaving Neovim for Zed](https://unixism.net/2024/08/leaving-neovim-for-zed/) (as summarized by search; the exact wording was not re-verified)

### Inferences
- Fyx's WASM per-extension sandbox avoids VS Code's shared-thread failure, provided the host enforces time and fuel budgets per extension call and renders asynchronously.
- A versioned WIT world (`fyx:ui@x.y`) with Neovim-style `since` and `deprecated` metadata and a "no removal before 2.0" promise directly addresses churn.
- Bus factor: an ecosystem where 5–6 of the top plugins come from one author (folke) is fragile. Fyx should ship first-party reference extensions for the top primitives (picker, which-key, flash-style jump) so the ecosystem does not depend on a single third party.
- Discoverability: Dotfyle (installs across real configs) shows that usage telemetry from configs beats stars. Fyx's extension registry could publish install counts and "used by N configs".

### Gaps
- I found no authoritative VS Code doc on extension host architecture. The source used is a wiki copy.
- The exact quote from the unixism.net post was not verified by fetch.

## Q6. Synthesis: primitives ranked by how many essential plugins need them, and API-design lessons

### Takeaway
Across the roughly 30 essential plugins (Neovim's top UI plugins, magit/forge/org/vertico-stack/avy/projectile/gptel/which-key in Emacs, Sublime LSP and SideBar), the most-depended-on primitives are, in order: (1) a fuzzy picker with preview and actions, (2) anchored decorations (signs, line highlights, inline/EOL virtual text), (3) buffers or panels as keyboard-navigable UI with typed sections and per-item keymaps, (4) floating popups, (5) async processes, streaming and timers, (6) key capture plus overlay labels and keymap introspection, (7) a host diff view, (8) status-line segments. Fyx already covers panels, the declarative tree, block decorations, diff and markdown hosts, and the planned quick-pick and timers. Its biggest gaps are signs and gutter, inline and EOL virtual text and conceal, modal key capture with overlay labels, keymap introspection, and virtual documents.

### Cited Findings
Consolidated top-30. Popularity sources are those cited in Q1–Q3; Zed coverage is from Q4.

| # | Plugin (editor) | Popularity | Does | Key primitives | Zed native? |
|---|---|---|---|---|---|
| 1 | vim-fugitive (nvim) | 21,828★ [GH](https://github.com/tpope/vim-fugitive) | Git porcelain | buffers-as-UI, virtual `fugitive://` docs, diff | partial |
| 2 | telescope.nvim (nvim) | 19,861★ [GH](https://github.com/nvim-telescope/telescope.nvim) | fuzzy picker | float, prompt, previewer, async sources, actions | partial |
| 3 | avante.nvim (nvim) | 18,198★ [GH](https://github.com/yetone/avante.nvim) | AI sidebar + apply | sidebar, streaming, inline diff (virt_lines) | yes (agent) |
| 4 | mini.nvim (nvim) | 9,669★ [GH](https://github.com/nvim-mini/mini.nvim) | 40+ modules (pick, files, clue, jump2d, diff, statusline) | float, extmarks, key capture, buffers-as-UI | partial |
| 5 | harpoon (nvim) | 9,244★ [GH](https://github.com/ThePrimeagen/harpoon) | pinned-file quick switch | float editable list, per-project state | no |
| 6 | snacks.nvim / picker (nvim) | 8,224★ [GH](https://github.com/folke/snacks.nvim) | picker, notifier, dashboard… | float, picker, notifications | partial |
| 7 | lualine (nvim) | 8,188★ [GH](https://github.com/nvim-lualine/lualine.nvim) | status line | statusline/winbar segments | no (fixed) |
| 8 | which-key (nvim) | 7,341★ [GH](https://github.com/folke/which-key.nvim) | key hints | keymap introspection, pending-key hook, popup | yes |
| 9 | gitsigns (nvim) | 7,188★ [GH](https://github.com/lewis6991/gitsigns.nvim) | hunk signs, blame, stage | signs, line highlight, EOL virt_text, virt_lines | mostly |
| 10 | oil.nvim (nvim) | 6,984★ [GH](https://github.com/stevearc/oil.nvim) | edit FS as buffer | editable buffer-as-UI, virtual URI, conceal | no |
| 11 | codecompanion (nvim) | 6,945★ [GH](https://github.com/olimorris/codecompanion.nvim) | AI chat + inline | chat buffer, streaming, inline diff | yes |
| 12 | trouble (nvim) | 6,915★ [GH](https://github.com/folke/trouble.nvim) | diagnostics/refs list | list/tree panel, extmarks | yes |
| 13 | obsidian.nvim (nvim) | 6,225★ + fork 2,250★ [GH](https://github.com/epwalsh/obsidian.nvim) | vault notes | picker, conceal/virt_text, completion | no |
| 14 | diffview (nvim) | 5,889★ [GH](https://github.com/sindrets/diffview.nvim) | multi-file diff + history | diff host, file tree panel, layouts | partial |
| 15 | noice (nvim) | 5,832★ [GH](https://github.com/folke/noice.nvim) | replaces cmdline/messages | ui_attach, float, notifications | n/a |
| 16 | neogit (nvim) | 5,678★ [GH](https://github.com/NeogitOrg/neogit) | magit clone | sectioned buffer, transient-like popups | partial |
| 17 | conform (nvim) | 5,403★ [GH](https://github.com/stevearc/conform.nvim) | formatters | save hook, minimal-diff edits, jobs | yes |
| 18 | render-markdown (nvim) | 5,256★ [GH](https://github.com/MeanderingProgrammer/render-markdown.nvim) | in-buffer md render | conceal, inline/overlay virt_text, decoration provider | partial |
| 19 | leap / flash (nvim) | 5,028★ / 4,280★ [leap](https://github.com/ggandor/leap.nvim) [flash](https://github.com/folke/flash.nvim) | labeled jumps | overlay labels, key capture loop | partial (#14801 open) |
| 20 | todo-comments (nvim) | 4,229★ [GH](https://github.com/folke/todo-comments.nvim) | TODO highlight/search | highlights, signs, picker | no |
| 21 | octo.nvim (nvim) | 3,410★ [GH](https://github.com/pwntester/octo.nvim) | GitHub PRs/issues | editable buffers, virt_text/signs threads, picker | no |
| 22 | nvim-dap-ui (nvim) | 3,389★ [GH](https://github.com/rcarriga/nvim-dap-ui) | debugger UI | panel layouts, tree, float | yes |
| 23 | neotest (nvim) | 3,122★ [GH](https://github.com/nvim-neotest/neotest) | test runner | signs, virt_text, tree panel, jobs | partial |
| 24 | overseer (nvim) | 1,954★ [GH](https://github.com/stevearc/overseer.nvim) | task runner | jobs, list panel, status segment | yes-ish |
| 25 | magit (Emacs) | 5.37M dl [MELPA](https://melpa.org/download_counts.json) | Git porcelain | sectioned buffers, transient menus | partial |
| 26 | org-mode (Emacs) | built-in | outlines, agenda, tasks | buffers-as-UI, folding, overlays | no |
| 27 | vertico+consult+embark+orderless+marginalia (Emacs) | 473k/779k/365k/482k/549k dl [MELPA](https://melpa.org/download_counts.json) | composable completion | completing-read protocol, annotators, actions | partial |
| 28 | avy (Emacs) | 2.56M dl [MELPA](https://melpa.org/download_counts.json) | labeled jumps | overlays + transient keymap | partial |
| 29 | projectile (Emacs) | 3.37M dl [MELPA](https://melpa.org/download_counts.json) | project nav | picker, project model | yes |
| 30 | forge (Emacs) | 811k dl [MELPA](https://melpa.org/download_counts.json) | GitHub/GitLab in magit | sectioned buffers | no |
| 31 | gptel (Emacs) | 270k dl [MELPA](https://melpa.org/download_counts.json), 3,573★ [GH](https://github.com/karthink/gptel) | LLM in any buffer | streaming into buffers, transient menu | yes (agent) |
| 32 | which-key (Emacs, now core) | 2.20M dl [MELPA](https://melpa.org/download_counts.json) | key hints | keymap introspection, popup | yes |
| 33 | LSP (Sublime) | 1,813★ [GH](https://github.com/sublimelsp/LSP) | LSP client | phantoms, popups, regions, quick panel | yes |
| 34 | SideBarEnhancements (Sublime) | 2.95M inst [PC](https://packagecontrol.io/browse/popular) | file ops on sidebar | sidebar context commands | yes |

### Inferences
**Primitive ranking.** Counts are approximate, from the table above (34 rows) and Q1's mapping. The last part of each item is Fyx's status.

1. **Picker / quick-pick:** async sources, fuzzy matching, live preview, multi-select and actions on the selected item. About 14 plugins: telescope, fzf-lua, snacks, mini.pick, harpoon, todo-comments, obsidian, octo, overseer, the vertico stack, projectile, Sublime quick panel, trouble-like lists, neogit. *Fyx: planned.* It should be protocol-first: sources, matcher, annotator and actions as separate extension points, Emacs-style. It also needs on-highlight preview like Sublime's `on_highlight`.
2. **Anchored decorations:** signs and gutter, line and number highlights, EOL and inline virtual text, overlay text, conceal. About 13 plugins: gitsigns, todo-comments, neotest, dap, trouble, render-markdown, obsidian, octo, flash, leap, avy, avante, codecompanion, Sublime regions and phantoms. *Fyx: block decorations only. Signs, EOL and inline virtual text, highlight ranges and conceal are the main gaps.* Adopt Neovim's model: one anchored-mark primitive with namespace ownership, priority and a viewport-scoped provider callback.
3. **Buffers or panels as UI:** keyboard-navigable lists and trees with typed sections, folding, per-item keymaps, act-on-thing-at-point and refresh that preserves state. About 12 plugins: fugitive, neogit, magit, forge, oil, org, octo, trouble, overseer, dap-ui, neotest summary, diffview file panel, codecompanion and avante chat. *Fyx: dock panels with a virtualized list and table exist.* Missing: per-row keymaps, vim-style navigation inside panels, and "editable buffer whose save applies a diff" (oil, octo) — ideally a host editor view backed by an extension-owned virtual document.
4. **Floating popups anchored to the cursor, a line or the editor:** hover output, previews, menus. About 10 plugins: telescope, which-key, noice, harpoon, gitsigns preview, dap hover, neotest output, snacks, mini, Sublime popups, posframe in Emacs. *Fyx: modal planned.* Should also support cursor- and anchor-relative, non-modal popovers.
5. **Async processes, streaming text and timers:** about 10 plugins: overseer, neotest, conform, gptel, avante, codecompanion, telescope live grep, magit, gitsigns, octo. *Fyx: timers planned.* Process exec exists via Zed capabilities. Streaming updates into panels and decorations are needed.
6. **Key capture and transient keymaps, with overlay labels and keymap introspection:** about 7 plugins: flash, leap, avy, mini.jump2d, which-key (nvim and Emacs), mini.clue, transient/neogit popups, hydra. *Fyx: missing.* This is the core of keyboard-centric plugins. It needs a "capture next N keys while the extension is active" mode, read-only keymap and context introspection, and labels drawn as overlay decorations over visible ranges.
7. **Host diff view:** about 6 plugins: diffview, fugitive, neogit, magit, avante, codecompanion, octo review. *Fyx: has it.* Should accept extension-supplied before/after content and per-hunk actions.
8. **Status-line / winbar segments:** about 5 plugins: lualine, overseer, noice, mini.statusline, doom-modeline. *Fyx: has it.*
9. **Virtual documents (custom URI schemes)** opened in a normal editor: about 5 plugins: fugitive, octo, oil, diffview, Emacs TRAMP-like handlers. *Fyx: missing.*
10. **Notifications and toasts:** about 4 plugins: nvim-notify, noice, snacks.notifier, mini.notify. *Fyx: not mentioned.* Cheap to add.
11. **Text-edit transactions with minimal diffs and save hooks:** conform, nvim-surround, oil. *Fyx: N/A as UI, but needed alongside it.*

**API-design lessons**
- **One anchor, many decoration kinds, plus namespaces** (Neovim extmarks). CRDT anchors already give Fyx the tracking. Expose per-extension namespaces with clear-all and priority, and a viewport-pull provider, to avoid pushing decorations for whole files.
- **Buffers as UI** (magit, dired, org, oil, fugitive): keyboard users want plugin UI navigable with the same motions, search and yank as text. A flat tree of buttons is not enough. Typed rows with per-type keymaps, "act on item at cursor", and folding are what make magit loved.
- **Protocol-first picker** (Emacs completing-read stack): make quick-pick a shared service where extensions contribute sources and actions, as embark and consult do, so one extension can add actions to another's candidates.
- **Restricted markup instead of webviews** (Sublime minihtml phantoms and popups): validates Fyx's theme-token tree. Webview demand in Zed (#21208, about 506 reactions) suggests an escape hatch may still be asked for.
- **Ship the API, even small** (Helix): three years of draft status fragments the community into forks. **UI surface drives variety** (Lapce, Zed): a WASI sandbox without UI yields only language servers.
- **Isolation per extension and budgets** (VS Code extension host): a single shared runtime lets one plugin freeze all the others.
- **Stability contract** (Neovim `since`, `deprecated_since`, "not removed until 2.0"): version the WIT world and publish an api_level.
- **IPC/CLI escape hatch** (Kakoune `kak -p`): lets external TUIs and scripts drive the editor without writing an extension.

### Gaps
- The per-primitive dependency counts are analyst judgments over a hand-picked plugin set, not measured (for example by grepping plugin sources for `nvim_buf_set_extmark` or `nvim_open_win`). A code-search count over the top-100 Dotfyle plugins would make the ranking rigorous.
- Install-based popularity for 2026 (Dotfyle full list, Neovim user survey) was not retrieved. Stars favor older plugins.
