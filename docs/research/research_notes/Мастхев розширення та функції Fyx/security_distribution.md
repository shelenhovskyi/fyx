# Editor-extension security, sandboxing, permissions and distribution (state as of Nov 2026)

Scope: evidence for which platform features Fyx (Zed fork, `fyx:ui` WASM components, a planned VS Code sidecar under Node `--permission`, and a possible own registry) must ship. Research date 2026-11-17. Many incident figures come from security-vendor blogs, and vendors often disagree on counts. Conflicts are flagged inline.

## 1. Which extension supply-chain incidents happened 2023–2026: vector, scale, fix

### Takeaway
The main threat in 2025–2026 is stolen or leaked publisher credentials used to push a malicious update to an already-trusted extension (GlassWorm, AsyncAPI/Shai-Hulud 2.0, Nx Console to GitHub breach, Cyberhaven in Chrome). Next come typosquats and clones gamed into search ranking (fake Solidity, about $500k stolen), and transitive installs via `extensionPack`/`extensionDependencies`. Malicious versions are often live for only minutes, yet still reach thousands of machines, because auto-update pushes them and nothing forcibly removes them afterwards. The targets are developer secrets: GitHub/npm/cloud tokens, crypto wallets and, since 2026, AI-agent configs such as `~/.claude/settings.json`.

### Cited Findings

**GlassWorm (Open VSX, with spillover to the VS Code Marketplace), Oct 2025 to at least Apr 2026**
- Oct 17, 2025: Koi Security reported a self-propagating campaign that hid its loader with invisible Unicode variation selectors, so reviewers and static analysers could not see it. It used Solana-blockchain C2, which is hard to take down — [heise](https://heise.de/-10790065); [Koi (original)](https://www.koi.ai/blog/glassworm-first-self-propagating-worm-using-invisible-code-hits-openvsx-marketplace)
- Install count conflict: about 35,800 downloads across 7 Open VSX extensions per Koi/heise — [heise](https://heise.de/-10790065); [securitylab.ru](https://www.securitylab.ru/news/564902.php). iTnews reported 7 extensions and 10,700+ downloads — [iTnews](https://www.itnews.com.au/news/hidden-glassworm-malware-spreads-through-infected-vs-code-extensions-621223). The Eclipse Foundation said 35,800 "overstates the actual number of affected users" because it includes bot and visibility-boosting downloads. Eclipse also disputed the "worm" label, saying the malware "did not autonomously propagate" — [Eclipse blog, 27 Oct 2025](https://blogs.eclipse.org/post/mika%C3%ABl-barbero/open-vsx-security-update-october-2025)
- Vector: Open VSX publishing tokens that developers had leaked in public repos. The malware then uses stolen Open VSX credentials to publish more malicious extensions. At least one infected extension also reached Microsoft's VS Code Marketplace — [Eclipse blog](https://blogs.eclipse.org/post/mika%C3%ABl-barbero/open-vsx-security-update-october-2025); [heise](https://heise.de/-10790065); [Techzine](https://www.techzine.eu/news/security/135634/invisible-malware-spreads-via-vs-code-extensions/)
- Open VSX declared the incident "fully contained and closed" on Oct 21, 2025. Koi then reported new waves, including infection spreading to GitHub repos — [Koi follow-up](https://www.koi.ai/blog/glassworm-returns-new-wave-openvsx-malware-expose-attacker-infrastructure); [Dark Reading](https://www.darkreading.com/cyberattacks-data-breaches/glassworm-returns-vs-code-extensions)
- Jan 30, 2026: attackers compromised the publishing credentials of legitimate Open VSX developer "oorzc" and pushed updates to 4 existing extensions. The loader profiles the machine before detonating — [The Hacker News, Feb 2026](https://thehackernews.com/2026/02/open-vsx-supply-chain-attack-used.html?m=0)
- Mar 2026: Socket found at least 72 more malicious Open VSX extensions since Jan 31. The new technique abuses the manifest fields `extensionPack` and `extensionDependencies`: a benign-looking extension pulls in the malicious loader transitively. Lures included Claude Code, Codex and Antigravity tooling — [Socket](https://socket.dev/blog/open-vsx-transitive-glassworm-campaign); [CSO Online](https://www.csoonline.com/article/4145579/open-vsx-extensions-hijacked-glassworm-malware-spreads-via-dependency-abuse.html)
- Apr 2026: Socket flagged 73 cloned "sleeper" extensions from newly created GitHub accounts. Most were dormant; 6 were active and delivering GlassWorm v2 — [Socket](https://socket.dev/blog/73-open-vsx-sleeper-extensions-glassworm); [SecurityWeek](https://www.securityweek.com/dozens-of-open-vsx-extension-clones-linked-to-glassworm-malware/amp/)
- Checkmarx documented a further evolution that hides staged malware behind runtime-rebuilt loaders — [Checkmarx Zero](https://checkmarx.com/zero-post/glassworm-targets-developer-ides-again-hiding-staged-malware-behind-runtime-rebuilt-loaders/)

**Leaked publisher tokens (Wiz, Oct 2025)**
- Wiz found 100+ valid VS Code Marketplace PATs and 30+ Open VSX access tokens leaked inside published extension packages. Eclipse revoked them — [The Hacker News](https://thehackernews.com/2025/10/eclipse-foundation-revokes-leaked-open.html); [Eclipse blog](https://blogs.eclipse.org/post/mika%C3%ABl-barbero/open-vsx-security-update-october-2025)

**Shai-Hulud npm worm and its spillover into extensions**
- Shai-Hulud 1 (mid-Sep 2025) was a self-replicating npm worm that stole tokens and republished packages. Counts range from "several dozen" to "hundreds" of packages — [Invicti](https://invicti.com/blog/web-security/shai-hulud-2-worm-supply-chain-attack-on-npm-dependencies); [Unit 42](https://unit42.paloaltonetworks.com/npm-supply-chain-attack/); [Decipher](https://decipher.sc/2025/11/24/new-shai-hulud-npm-worm-emerges)
- Shai-Hulud 2.0 (Nov 21–23, 2025) hit 621 npm packages per Check Point (796 per other counts), about 25,000 GitHub repos and 487 organisations. Check Point counted 14,206 leaked secrets, 2,485 of them still valid — [Check Point](https://blog.checkpoint.com/research/shai-hulud-2-0-inside-the-second-coming-the-most-aggressive-npm-supply-chain-attack-of-2025/)
- AsyncAPI (Nov 2025): the first known compromise of the 2.0 wave. Attackers used npm and Open VSX tokens that had sat in GitHub repo secrets for years to publish a malicious AsyncAPI VS Code extension on Open VSX. The token was revoked quickly and the version was unpublished within minutes — [Aikido](https://www.aikido.dev/blog/vs-code-extension-github-breach); [AsyncAPI postmortem](https://www.asyncapi.com/blog/shai-hulud-postmortem)
- Key lesson from Wiz: the extension was pulled on Nov 26, yet it stayed active on developer machines because no higher version existed to trigger an auto-update. Most infections were cleared only when a clean 1.1.0 shipped on Dec 24 — [Wiz](https://www.wiz.io/blog/snipping-the-long-tail-of-shai-hulud-2-0)

**Nx Console to GitHub internal breach (VS Code Marketplace, May 18, 2026)**
- Malicious `nrwl.angular-console` 18.95.0 was published with a contributor's GitHub token scraped in an earlier supply-chain attack (reported as the TanStack compromise). It was live for about 11–18 minutes; sources conflict on the exact window. It fetched its payload from an orphan commit in the official nrwl/nx repo. It harvested GitHub, npm, AWS, Vault, Kubernetes and 1Password secrets and targeted `~/.claude/settings.json` — [Aikido](https://www.aikido.dev/blog/vs-code-extension-github-breach); [Help Net Security](https://www.helpnetsecurity.com/2026/05/20/github-breached-teampcp/); [OX Security](https://www.ox.security/blog/teampcp-strikes-again-how-a-trojan-vs-code-extension-brought-down-github/)
- GitHub said an employee device was compromised by a poisoned third-party VS Code extension. It called the attacker's claim of about 3,800 exfiltrated internal repos "directionally consistent". Attribution to TeamPCP comes from vendors, not from GitHub — [VentureBeat](https://venturebeat.com/security/github-confirms-3800-repos-stolen-poisoned-vs-code-extension-supply-chain-worm-microsoft-python-sdk); [Help Net Security](https://www.helpnetsecurity.com/2026/05/20/github-breached-teampcp/)

**Typosquat / ranking abuse: fake "Solidity Language" for Cursor (Open VSX, Jun–Jul 2025)**
- A Russian blockchain developer lost about $500k in crypto after installing a fake "Solidity Language" extension from Open VSX, which is Cursor's extension source. The fake ranked 4th in search versus 8th for the real "solidity", helped by a more recent update date. It dropped a Quasar backdoor and a stealer; SC World says PureLogs — [Kaspersky press release](https://www.kaspersky.com/about/press-releases/kaspersky-uncovers-500k-crypto-heist-through-malicious-packages-targeting-cursor-developers); [SC World](https://www.scworld.com/news/fake-visual-studio-code-extension-for-cursor-led-to-500k-theft)
- About 50–54k downloads before removal on Jul 2, 2025. A republished copy showed about 2M downloads, which Kaspersky judged likely inflated. The attacker republished after takedown — [Snyk](https://snyk.io/pt-BR/blog/cursor-ide-malware-extension-compromise-in-usd500k-crypto-heist/); [SC World](https://www.scworld.com/news/fake-visual-studio-code-extension-for-cursor-led-to-500k-theft)
- Other Open VSX impersonations: a fake Prettier extension (Nov 2025, few downloads) and a fake Solidity extension in 2024 — [aviatrix summaries](https://aviatrix.ai/threat-research-center/fake-solidity-vscode-extension-supply-chain-attack-2024); [Visual Studio Magazine, Dec 2025](https://visualstudiomagazine.com/articles/2025/12/08/threat-actors-keep-weaponizing-vs-code-extensions.aspx)

**Ecosystem-level studies**
- An academic analysis of the VS Code ecosystem (Nov 2024) found that 7% of extensions tagged for harmful content carried the domain-verified badge — [arXiv 2411.07479](https://arxiv.org/html/2411.07479v1)
- OX Security reported that flawed checks let publishers add functionality while keeping the verified badge (2025; fix status unknown) — [OX Security](https://www.ox.security/blog/can-you-trust-that-verified-symbol-exploiting-ide-extensions-is-easier-than-it-should-be/)

**Zed**
- No reports of a malicious Zed extension were found. Searches returned only VS Code/Open VSX cases — [search coverage, e.g. Visual Studio Magazine](https://visualstudiomagazine.com/articles/2025/12/08/threat-actors-keep-weaponizing-vs-code-extensions.aspx). An informal 2025 community comment notes that Zed language-server extensions download and run native LSP binaries outside the Wasm sandbox — [Lemmy thread](https://lemmy.amethyst.name/post/241351/124973). An older Zed GitHub discussion on extension supply-chain security exists — [zed discussion #10375](https://github.com/zed-industries/zed/discussions/10375)

**Browser-extension analogues**
- Cyberhaven (Dec 24–26, 2024): a phished developer granted OAuth access to the Chrome Web Store account, and a malicious update auto-installed for about 400k users. It was detected about 30 hours later and removed within an hour. The broader campaign hit several other extensions — [Sekoia](https://blog.sekoia.io/targeted-supply-chain-attack-against-chrome-browser-extensions/)
- ShadyPanda (disclosed Dec 2025 by Koi): a 7-year campaign, 4.3M installs in total. Extensions were legitimate at first and turned malicious via updates in mid-2024. WeTab alone had about 3M installs — [The Hacker News](https://thehackernews.com/2025/12/shadypanda-turns-popular-browser.html); [SC World](https://www.scworld.com/news/shadypanda-exploited-chrome-edge-browser-extensions-for-7-years)
- Users who already had the extension were not notified after removal (secondary claim) — [Hive Security](https://hivesecurity.gitlab.io/blog/browser-extension-notification-crisis/)

**Analogous plugin ecosystem: Obsidian**
- A 2026 social-engineering campaign used Obsidian plugins (Shell Commands) and plugin sync to deliver malware. That campaign preceded Obsidian's review overhaul — [Penligent](https://www.penligent.ai/hackinglabs/obsidian-shell-commands-abuse-shows-a-new-malware-playbook/); [Obsidian blog](https://obsidian.md/blog/future-of-plugins/)

### Inferences
- The dominant vector is a malicious update to a trusted ID, not a malicious new extension. Install-time review and prompts alone do not cover it. The platform needs update-time controls: signing tied to the publisher key, diffing of the capability set on update, delayed or staged rollout, and a remote kill switch.
- "Remove from registry" does not clean infected machines (Wiz/AsyncAPI). The host needs a block list that disables or uninstalls already-installed versions, as VS Code does, and a way to force a downgrade.
- Transitive installs (`extensionPack`/dependencies) are an attack surface. Every dependent extension should go through the same consent flow and appear in the UI.
- Fyx's VS Code adapter will inherit the Open VSX threat model wholesale (GlassWorm, typosquats). That path needs the strictest defaults.
- AI-agent config files are now explicit targets. Agent-related capabilities (`agent:review`, access to agent settings and tokens) should be among the most tightly guarded.

### Gaps
- No primary Koi GlassWorm write-up was fetched; the counts rely on heise, securitylab and Eclipse.
- No primary GitHub statement was fetched for Nx Console; figures come from VentureBeat, Help Net Security and vendors.
- Microsoft has published no aggregate count of malicious extensions removed from the Marketplace in 2025–2026 that this research found.
- No Zed-specific incident was found. This is absence of evidence only.

## 2. How platforms defend (VS Code, Open VSX, JetBrains, Chrome, Zed, Node/Deno, Obsidian, Raycast, Figma)

### Takeaway
Registries defend at publish time: AV and dynamic scanning, secret scanning, name-squat protection, publisher verification, signing, block lists. Hosts defend at install time: a publisher-trust prompt, enterprise allowlists, Workspace Trust. Neither VS Code nor Obsidian sandboxes extensions at runtime. Zed has a Wasm sandbox plus a coarse user-level capability list (`process:exec`, `download_file`, `npm:install`), with no per-extension prompt. Node's permission model is explicitly "not a security boundary".

### Cited Findings

**VS Code / Visual Studio Marketplace**
- Each package and update is scanned with "several antivirus engines" and is not public until clean. Extensions also run in a "clean room VM" for dynamic analysis during review — [VS Code docs: Extension runtime security](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security)
- Name-squatting protection reserves the names of official publishers (Microsoft, RedHat, GitHub Copilot). The Marketplace monitors download and usage patterns for anomalies — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security)
- Secret scanning blocks publishing when API keys are found; `vsce` also scans `.env` files during packaging — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security)
- Signing: the Marketplace signs every extension at publish time, and VS Code verifies the signature at install for "integrity and the source" — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security). The limit is that a valid signature only proves these are the bytes the publisher uploaded, so a compromised account still yields validly signed malware — [decryptiondigest summary](https://www.decryptiondigest.com/blog/malicious-vscode-extension-enterprise-defense)
- Verified publisher (blue check) requires proven domain ownership and at least 6 months in good standing — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security)
- Kill switch: extensions confirmed malicious, or with vulnerable dependencies, go on a block list. If installed, "it's automatically uninstalled by VS Code" — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security)
- Publisher trust prompt (since VS Code 1.97, early 2025): first install from a new third-party publisher shows a dialog. Trusting a pack or an extension with dependencies also trusts the dependent publishers. Publishers installed earlier are auto-trusted, and CLI installs are not auto-trusted. The list is managed via "Extensions: Manage Trusted Extensions Publishers" — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security)
- Enterprise allowlist: the `extensions.allowed` setting, enforced via the `AllowedExtensions` policy on managed devices, reported as introduced in VS Code 1.96. Secondary sources conflict on the key name; the VS Code docs link to the enterprise page "Configure allowed extensions" — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security); [Socket glossary](https://socket.dev/glossary/ide-extension-security)
- No runtime sandbox: the extension host "has the same permissions as VS Code itself". It can read and write files, make network requests, run processes and change settings. Workspace Trust only gates whether code in a folder can be executed without approval — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security)

**Open VSX (Eclipse Foundation)**
- Oct 2025 response: leaked tokens revoked; a new token prefix designed "in collaboration with MSRC" so secret scanners can detect it; shorter default token lifetimes; faster revocation tooling; automated scanning "at the time of publication" for malicious patterns and embedded secrets; intelligence sharing with VS Code and the forks — [Eclipse blog, 27 Oct 2025](https://blogs.eclipse.org/post/mika%C3%ABl-barbero/open-vsx-security-update-october-2025)
- Token prefix: reports cite `ovsxat_`, and one article misspells it `osvxat_`. Verify in the docs — [The Hacker News](https://thehackernews.com/2025/10/eclipse-foundation-revokes-leaked-open.html)
- A statement of work for short-term security improvements specifies malware, name-squatting and secret checks. Extensions publish only after passing every check; failures are quarantined with admin notification — [Eclipse SOW PDF](https://www.eclipse.org/lists/openvsx/pdfn2OhDfOTiF.pdf); [heise](https://www.heise.de/en/news/Open-VSX-Eclipse-Foundation-Draws-Consequences-from-GlassWorm-Attack-10965519.html)
- Trusted publishing (OIDC from CI, no stored token) is now the recommended route, and maintainers are told to delete stored tokens from CI secrets — [Alpha-Omega](https://alpha-omega.dev/blog/fewer-keys-under-the-doormat-why-trusted-publishing-matters-for-open-vsx/)

**JetBrains Marketplace**
- Every new plugin and every update goes through verification and approval "at the sole discretion" of the Marketplace team. JetBrains may re-review and withdraw approval at any time — [JetBrains Approval Guidelines](https://plugins.jetbrains.com/docs/marketplace/jetbrains-marketplace-approval-guidelines.html)
- The review has four steps: automatic checks at upload, Plugin Verifier compatibility checks, automated tests and manual moderation. Manual review usually takes up to 2 business days. Updates get lighter checks than new plugins (forum report) — [JetBrains docs](https://plugins.jetbrains.com/docs/marketplace/release-plugin.html); [JetBrains forum](https://intellij-support.jetbrains.com/hc/en-us/community/posts/12884325007122/comments/12958806294162)
- The Q1 2026 newsletter mentions a new Approval Criteria clause and improved defaults for plugin signing and verification in the Gradle tooling. Details were not retrieved — [JetBrains Platform blog, Apr 2026](https://blog.jetbrains.com/platform/2026/04/busy-plugin-developers-newsletter-q1-2026/)

**Chrome Manifest V3**
- `optional_host_permissions` are granted at runtime, not at install. Adding host permissions or content-script matches in an update triggers a new install-time warning, and the extension is disabled until the user accepts — [Chrome docs: declare permissions](https://developer.chrome.com/docs/extensions/develop/concepts/declare-permissions)
- `activeTab` grants temporary access to the current tab only on user invocation and shows no install warning. Users can set site access per extension: on click, on specific sites or on all sites — [Chrome user privacy](https://developer.chrome.com/docs/extensions/mv3/user_privacy); [Chrome runtime host permissions guide](https://developer.chrome.com/docs/extensions/mv2/runtime_host_permissions/)
- Firefox supports `optional_host_permissions` since release 128 — [MDN](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/optional_permissions)

**Zed**
- Extensions are Wasm. Their privileged operations are gated by three capabilities. `process:exec` (`command`, `args`) is used by `zed_extension_api::process::Command`. `download_file` (`host`, `path` segments) restricts downloads to one host or one repo. `npm:install` (`package`) restricts installs to a named package. Users restrict them via the `granted_extension_capabilities` setting. An empty list removes all capabilities and "will likely make many extensions non-functional"; a call without the capability errors — [Zed docs: Extension Capabilities](https://zed.dev/docs/extensions/capabilities)
- By default no restrictions apply: the default grant is wildcard — [search summary of Zed docs](https://zed.dev/docs/extensions/capabilities); [All Settings](https://zed.dev/docs/reference/all-settings?highlight=install+dev+extension). The Zed page itself shows wildcard examples but does not state the default explicitly.
- There is no per-extension consent prompt and no per-extension capability manifest shown to users. The setting is global, not per extension — [Zed docs](https://zed.dev/docs/extensions/capabilities)
- Example of real use: the Java extension needs `process:exec` to launch the JVM, and authorizes a specific `-version` argument pattern for JDK discovery — [DeepWiki on zed-extensions/java](https://deepwiki.com/zed-extensions/java/2-extension-configuration)

**Node.js permission model (relevant to the Fyx VS Code sidecar)**
- The model was added in v20.0.0 and has been stable since v23.5.0 / v22.13.0. Under `--permission` it restricts fs, network, child processes, workers, native addons, WASI, FFI and the inspector. Grants: `--allow-fs-read`, `--allow-fs-write`, `--allow-child-process`, `--allow-worker`, `--allow-addons`, `--allow-wasi`, `--allow-ffi`, `--allow-net`. Runtime checks use `process.permission.has()`; `process.permission.drop()` is irreversible — [Node v26.5.1 permissions docs](https://r2.nodejs.org/dist/v26.5.1/docs/api/permissions.html)
- Node itself calls the model a seat belt, not a security boundary. Node "trusts any code it is asked to run". Symlinks are followed outside granted paths. Existing file descriptors and other modules (e.g. `node:sqlite`) bypass fs checks. Permissions do not inherit into workers or child processes. `process._debugProcess()` is ungated, so a sandboxed process can open the inspector of another same-user Node process. Node recommends separate OS users or OS-level isolation — [Node v26.5.1 docs](https://r2.nodejs.org/dist/v26.5.1/docs/api/permissions.html); [Node v24.18.0 docs](https://r2.nodejs.org/dist/v24.18.0/docs/api/permissions.html)
- Version note: `--allow-net` appears in the v26 docs. Whether Node 24 LTS (24.18 installed on this machine) restricts network under `--permission` was not confirmed — [Node v24.18.0 docs](https://r2.nodejs.org/dist/v24.18.0/docs/api/permissions.html)

**Obsidian (May 2026 overhaul)**
- A new Community hub runs automated checks on every plugin and theme version: policy, code quality, known vulnerabilities and malware scanning. Manual review of initial submissions continues. Plugins whose latest release fails automated review are reportedly delisted and made uninstallable after Oct 30, 2026 — [Obsidian blog: The future of plugins](https://obsidian.md/blog/future-of-plugins/); [GIGAZINE](https://gigazine.net/gsc_news/en/20260513-obsidian-plugin-future/); [WindowsForum](https://windowsforum.com/news/obsidian-plugin-safety-scorecards-audit-community-plugins-before-the-october-30-deadline.449302/)
- Plugins remain unsandboxed: they can access files, the network and install programs. Obsidian says it is too small to manually review every release — [thecodersblog](https://thecodersblog.com/obsidian-plugin-security-vulnerability-2026/); [Obsidian blog](https://obsidian.md/blog/future-of-plugins/)

**Raycast**
- Store submissions are pull requests to the open-source `raycast/extensions` monorepo. Raycast and the community review the source; CI validates the manifest, assets, author and build. Raycast rebuilds from source — [Raycast: prepare for store](https://developers.raycast.com/basics/prepare-an-extension-for-store); [Raycast guidelines](https://manual.raycast.com/extensions-guidelines)
- Enterprise and Teams can set an extensions allow-list — [Raycast Extensions Allow-List](https://manual.raycast.com/enterprise/extensions-allow-list.md)
- Raycast's own pages disagree on the runtime model. The security docs say extensions can access the file system; a blog post describes host-mediated APIs — [Raycast security](https://developers.raycast.com/information/security); [Raycast blog](https://www.raycast.com/blog/how-raycast-api-extensions-work.md)

### Inferences
- The best-documented runtime isolation among mainstream editors is Zed's Wasm sandbox plus capabilities. Fyx's WIT capability model is already stricter than Zed's: per-extension, declared and host-enforced. Fyx's gap versus VS Code is on the registry and host-ops side: signing, block list or kill switch, publisher trust, allowlist policy and secret and malware scanning.
- Treat Node `--permission` for the VS Code sidecar as defence-in-depth only. Pair it with OS isolation: a separate low-privilege user, Windows AppContainer or job objects, and a filtered environment with no inherited tokens. Fyx must also block `--allow-child-process` by default, because child processes escape the model.
- Chrome's "optional permissions + re-prompt on new permissions in update + disable until accepted" is the closest proven pattern for update-time consent.

### Gaps
- Figma's plugin review and `networkAccess.allowedDomains` manifest model was not retrieved, so the evidence is missing.
- Deno permission model specifics (`--allow-*`, prompts, `--deny-*`) were not retrieved in this pass.
- JetBrains plugin signing details (certificate chain, IDE verification behaviour) were not retrieved.
- VS Code "restricted mode" specifics and the exact `extensions.allowed` syntax were not retrieved from the enterprise docs page.

## 3. What permission UX actually works (install vs runtime, granularity, fatigue)

### Takeaway
Install-time permission lists are largely ignored. In a 2012 study only 17% of Android users paid attention and 3% understood them. Repeated warnings habituate users, and the habituation carries over to similar dialogs. Fewer, contextual runtime requests for genuinely sensitive actions work better. Low-risk capabilities should be auto-granted silently, and the evidence suggests reserving prompts for rare high-risk grants.

### Cited Findings
- Felt et al. (SOUPS 2012): only 17% of participants paid attention to permissions at install; 3% of internet survey respondents answered all three comprehension questions correctly; 42% of lab participants did not know permissions existed. The authors conclude that Android install-time warnings "do not help most users make correct security decisions" — [Berkeley tech report EECS-2012-26](https://www2.eecs.berkeley.edu/Pubs/TechRpts/2012/EECS-2012-26.pdf); [SOUPS '12 summary](https://blues.cs.berkeley.edu/?p=85)
- Felt et al., "How to Ask for Permission": habituation was observed for Windows UAC, Android install warnings and browser warnings. Users pay less attention to each subsequent warning, and the effect carries to similar warnings. Reducing the number of security decisions should reduce habituation. The paper recommends auto-granting low-risk, reversible permissions, runtime dialogs for rare risky ones, and install-time only for the rest — [UMD course copy of the paper](https://www.cs.umd.edu/class/fall2022/cmsc614/papers/android-permissions-attention.pdf)
- Wijesekera et al. (2015), "Android Permissions Remystified": a field study that motivates contextual, runtime decisions. Most earlier studies predate Android runtime dialogs — [arXiv 1504.03747](https://arxiv.org/pdf/1504.03747.pdf); [Bongard et al. 2022](https://www.unibw.de/usable-security-and-privacy/publikationen/pdf/bongard2022necessary.pdf/@@download/file/bongard2022necessary.pdf)
- Chrome's design choices follow the same direction. `activeTab` shows no install warning and grants access only on user gesture. Optional host permissions are requested at runtime. New permissions in an update re-prompt — [Chrome user privacy](https://developer.chrome.com/docs/extensions/mv3/user_privacy); [Chrome declare permissions](https://developer.chrome.com/docs/extensions/develop/concepts/declare-permissions)
- VS Code moved from no prompt to a single coarse publisher-trust prompt (1.97), not a per-permission one — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security)
- Zed relies on a settings file instead of prompts. It is off by default (wildcard grants), so in practice nobody restricts anything — [Zed docs](https://zed.dev/docs/extensions/capabilities)

### Inferences
- Suggested tiering for Fyx capabilities:
  - Silent, shown only on the details page: `ui:status-bar`, `ui:panel`, `ui:editor-decorations`.
  - Install-time disclosure with one confirm: `workspace:read-buffers`.
  - Explicit runtime or first-use consent, scoped and revocable: `process:sidecar`, network/download, `agent:review`, and anything that reads secrets or agent config.
- The update path must diff capabilities. If a new version requests a new high-risk capability, hold the update and prompt, Chrome-style. This directly addresses the compromised-update vector in section 1.
- Combine the consent with a publisher-trust step (VS Code style) for the first install from a publisher. Put it in the same dialog to avoid stacking prompts.

### Gaps
- No quantitative study specific to IDE or editor extension permission prompts was found.
- No Chrome-extension-specific comprehension study was retrieved in this pass.

## 4. Open VSX governance and sustainability; Microsoft Marketplace terms for forks

### Takeaway
Microsoft's Marketplace terms (Jan 2025 revision) restrict offerings to "In-Scope Products and Services", meaning Microsoft and GitHub products. Forks therefore cannot legally use it, and Microsoft enforced this against Cursor in April 2025. Open VSX, run by the Eclipse Foundation under a Working Group, is the de facto registry for VSCodium, Cursor, Windsurf and Theia. It is under financial strain: traffic grew 4x, funding stayed flat, and from 2025–2026 it imposes rate limits and offers a paid managed tier. A Fyx VS Code adapter would depend on Open VSX and must respect its limits, or mirror it.

### Cited Findings
- Microsoft Visual Studio Marketplace Terms of Use (Last Updated January 2025): "Marketplace Offerings are intended for use only with In-Scope Products and Services and you may not install, reverse-engineer, import or use Marketplace Offerings in products and services except for the In-Scope Products and Services." A separate clause covers offerings published by Microsoft or GitHub — [Marketplace & NuGet Terms of Use](https://www-1.nuget.org/policies/Terms)
- In-Scope products are listed as Visual Studio, VS Code, GitHub Codespaces, Azure DevOps and Azure DevOps Server, plus successors and affiliates such as GitHub — [DevClass, Apr 2025](https://www.devclass.com/development/2025/04/08/vs-code-extension-marketplace-wars-cursor-users-hit-roadblocks/1629343); [Terms copy](https://www-0.nuget.org/policies/Terms)
- Individual Microsoft extension licences are narrower. The C# Dev Kit may be used only with VS Code, vscode.dev, Codespaces and successor Microsoft/GitHub products — [C# Dev Kit license](https://marketplace.visualstudio.com/items/ms-dotnettools.csdevkit/license)
- Enforcement: in April 2025 Cursor users found that Microsoft extensions such as C/C++ stopped working, a sign of stricter enforcement — [DevClass](https://www.devclass.com/development/2025/04/08/vs-code-extension-marketplace-wars-cursor-users-hit-roadblocks/1629343). A reported Sept 2025 revision adding an explicit import ban is unconfirmed against Microsoft's text — [GitAutoReview](https://gitautoreview.com/blog/open-vsx-ai-code-review-cursor-vscodium-windsurf)
- Cursor made Open VSX the default for its marketplace around June 2025 — [Kaspersky press release](https://www.kaspersky.com/about/press-releases/kaspersky-uncovers-500k-crypto-heist-through-malicious-packages-targeting-cursor-developers)
- Open VSX was saved from closure by a new Eclipse Working Group in June 2023 — [DevClass, Jun 2023](https://www.devclass.com/development/2023/06/27/open-vsx-alternative-to-vs-code-marketplace-saved-from-closure-by-new-eclipse-working-group/1625540)
- An Open VSX outage of about 24 hours (Apr 2025) affected the forks that rely on it — [Hacker News thread](https://news.ycombinator.com/item?id=43785039)
- Sustainability: download traffic grew 4x in a year, mostly from AI-native tools, while funding stayed flat. Eclipse names Cursor and Windsurf among beneficiaries that have "not always stepped up" — [Eclipse Foundation blog, 23 Sep 2025](https://eclipse-foundation.blog/2025/09/23/businesses-built-on-open-infrastructure-have-a-responsibility-to-sustain-it/); [Eclipse: rate limiting](https://blogs.eclipse.org/post/christopher-guindon/scaling-open-vsx-registry-responsibly-rate-limiting)
- Rate limits: free individual use is capped at up to 3 requests/s, or 10,800/hour sustained. Open-source projects can be allow-listed; heavy commercial consumers are steered to a paid Managed Registry. One 2026 GitHub issue complains that usage cannot be tracked and describes an aggressive sales push — [Eclipse rate-limit post](https://blogs.eclipse.org/post/christopher-guindon/scaling-open-vsx-registry-responsibly-rate-limiting); [Rate-limit SOW](https://www.eclipse.org/lists/openvsx/pdfW68nr84QJo.pdf); [VMblog Q&A](https://vmblog.com/qa/why-the-eclipse-foundation-introduced-the-open-vsx-managed-registry-vmblog-qa/); [open-vsx.org issue #10550](https://github.com/EclipseFdn/open-vsx.org/issues/10550)
- Scale and backing: Open VSX passed 300M monthly downloads. AWS provides infrastructure credits and operational collaboration; long-term funding is "still being worked out" — [Eclipse newsroom](https://newsroom.eclipse.org/news/announcements/open-vsx-registry-surpasses-300-million-monthly-downloads-industry-leaders-back); [The New Stack](https://thenewstack.io/open-vsx-aws-investment/); [Eclipse: hardening Open VSX](https://blogs.eclipse.org/post/denis-roy/hardening-open-vsx-registry-keeping-it-reliable-scale)

### Inferences
- The Fyx VS Code adapter must use Open VSX, never the Microsoft Marketplace. Microsoft-published extensions (C/C++, C# Dev Kit, Pylance, Remote) are licence-locked even if installed from a .vsix.
- An editor product with many users should plan a caching proxy or mirror to stay within the rate limits, or budget for Eclipse membership or the Managed Registry.
- Open VSX's malware track record (section 1) means Fyx should layer its own policy on Open VSX-sourced extensions: block list sync, a quarantine age before install, and publisher-trust prompts.

### Gaps
- Microsoft's current Marketplace terms page (aka.ms/VSMarketplace-TOU) was not fetched directly; the quotes come from the NuGet-hosted copy.
- No explicit Open VSX terms clause for fork usage was retrieved; the Open VSX publisher agreement was not fetched.
- Whether Cursor or Windsurf signed commercial Open VSX agreements is unknown; no source was found.

## 5. Zed extension registry: how it works, review, terms for forks

### Takeaway
Zed's registry is the GitHub repo `zed-industries/extensions`, where every submission is reviewed by pull request ("not all of them make the cut"). Extensions must carry an approved licence. Zed's Terms of Service (Mar 2, 2026) don't mention extensions at all. They do require accessing the Service "only through the mechanisms designated by Zed" and forbid using non-public APIs. A fork calling Zed's extension API endpoints is at least legally ambiguous. The extension source code itself is open-licensed per extension.

### Cited Findings
- Registry: the `zed-industries/extensions` repo is the central home for extensions; about 2.9k forks. "Every submission is reviewed, and not all of them make the cut". Submitting is "the first step, not a guarantee". The repo has `extensions.toml`, an `extensions/` folder and a Danger-based PR check (`dangerfile.ts`) — [GitHub: zed-industries/extensions](https://github.com/zed-industries/extensions)
- The registry reportedly rejects extensions without an approved licence in `extension.toml` (third-party guide) — [skillsmp guide](https://skillsmp.com/skills/pr-pm-prpm-claude-skills-creating-zed-extensions-skill-md)
- Zed ToS (Last Updated Mar 2, 2026):
  - §2.2: "Customer agrees to access the Service only through the mechanisms designated by Zed".
  - §2.4(b) prohibits seeking "to access non-public APIs associated with the Service".
  - §2.4(a) prohibits providing access to the Service to third parties.
  - Open-source components are governed by their own licences (Apache-2.0, GPL-3.0), and restrictions "will not apply to such Open Source Component".
  - Extensions, the registry and api.zed.dev are not mentioned.
  — [Zed Terms of Service](https://zed.dev/terms)
- Zed's own framing: the app distributed by Zed Industries and Zed-operated services fall under the ToS, while source code falls under open-source licences — [Zed software overview](https://zed.dev/software-overview); [Zed on X](https://x.com/zeddotdev/status/2028886725150327070)
- The ToS overhaul blog explains the changes, and an HN thread shows community concern about licence-agreement clauses — [Zed blog: terms update](https://zed.dev/blog/terms-update); [HN discussion](https://news.ycombinator.com/item?id=47953501)
- Zed also runs a separate ACP (agent) registry; the extension format became the basis for its schema — [Zed blog: ACP Registry](https://zed.dev/blog/acp-registry)
- Community comment (informal): Zed-team extensions are vetted, community extensions vary, and users should review source when installing outside the main registry — [zed discussion #10375](https://github.com/zed-industries/zed/discussions/10375)

### Inferences
- Rebuilding from source is legally clean. Fyx could index the open-source extension repos listed in `zed-industries/extensions`, since each extension has its own OSS licence, build them in its own CI and host them on its own registry. That avoids calling Zed's hosted API, which plausibly counts as "the Service" or a "non-public API" under §2.2/§2.4(b). This is an inference, not legal advice.
- Zed's PR-review model resembles Raycast's (source in a public repo, CI build by the platform). It is a cheap way to get reviewable, reproducible builds without a binary-upload pipeline, and it suits Fyx's early scale.

### Gaps
- The Zed Publishing Guidelines and the CI build pipeline (who builds the Wasm, where artifacts are hosted, how updates are pushed) were not retrieved.
- No explicit Zed statement allows or prohibits forks using its extension API or registry.
- How Zed handles post-merge malicious updates (re-review per version bump, or not) is not documented in what was retrieved.

## 6. WASM component-model maturity in 2026: WASI 0.3, resource limits, Wasmtime advisories

### Takeaway
WASI 0.3.0, with native async (`stream<T>`, `future<T>`, async funcs in the canonical ABI), was ratified on June 11, 2026. Wasmtime 46 is the first release implementing it with async on by default; WASI 1.0 is the next target. The sandbox is strong but not perfect. 2026 brought Wasmtime's largest-ever advisory batch: 12 advisories on Apr 9, 2026, including 2 Critical sandbox escapes, mostly aarch64 and Winch, 11 of them found by LLM tools. More Cranelift escapes followed in October 2026, plus a wasmtime-wasi filesystem escape. Fyx must track Wasmtime patch releases closely.

### Cited Findings
- WASI 0.3.0 was ratified on June 11, 2026, rebasing WASI onto Component Model async primitives. The design is completion-based and replaces `wasi:io` polling. Wasmtime 46 is the first release with the final spec, async enabled by default. Wasmtime 43–45 implement the 0.3.0-rc-2026-03-15 snapshot; 41–42 had earlier snapshots — [Bytecode Alliance: WASI 0.3 Launched](https://bytecodealliance.org/articles/WASI-0.3); [wasi.dev WASI 0.3](https://wasi.dev/releases/wasi-p3)
- Early-2026 blogs describing a WASI 0.3 "release" in Wasmtime 37 refer to previews — [progosling](https://progosling.com/en/dev-digest/2026-02/wasi-0-3-wasmtime-37-native-async). A reported WASI 0.3.1 (Aug 11, 2026) is unconfirmed against official sources — [CODERCOPS blog](https://blog.codercops.com/blog/wasi-03-native-async-webassembly-server-2026)
- WASI 1.0 is targeted for late 2026 or early 2027 (secondary source) — [devnewsletter State of WebAssembly 2026](https://devnewsletter.com/p/state-of-webassembly-2026/)
- April 9, 2026 Wasmtime advisories:
  - 12 advisories, the largest batch ever, triple the whole of 2025. 11 were found by the team with LLM-based tools.
  - Critical: CVE-2026-34987 (Winch backend sandbox-escaping memory access, aarch64) and CVE-2026-34971 (aarch64 Cranelift miscompiled heap access, a lowering bug introduced in 32.0.0).
  - Moderate: several component-model string transcoding OOB reads and writes and panics (CVE-2026-34941, -35195, -34942), plus a `flags` lifting panic (-34943).
  - Low: pooling-allocator data leakage between instances (CVE-2026-34988).
  - Fixed in 43.0.1, 42.0.2, 36.0.7 and 24.0.7. Cranelift on x86_64 was not affected by the Criticals.
  - Non-security fixes are not backported. There is no continuous aarch64 fuzzing, because OSS-Fuzz lacks runners.
  - Planned responses: continuous aarch64 fuzzing, Cranelift formal verification in CI, and mandatory LLM-based bug hunting for new tier-1 features.
  — [Bytecode Alliance: April 9, 2026 Security Advisories](https://bytecodealliance.org/articles/wasmtime-security-advisories)
- October 2026: two more Cranelift sandbox escapes. CVE-2026-115889 affects all architectures (main fixed since 43.0.0; 36.x backport in 36.0.19). CVE-2026-115893 is ARM64 `br_table`, and hosts patched in April are still affected. Reported fixed versions are 36.0.19, 48.0.7, 49.0.4 and 50.0.0. No in-the-wild exploitation or CISA KEV listing as of Oct 26, 2026. Sources disagree on details, and the 6-digit CVE numbers are unusual; verify against GHSA-897f-g5gm-fmxm / GHSA-mghf-wjfp-p5xm — [The Hacker News, Oct 2026](https://thehackernews.com/2026/10/two-critical-wasmtime-flaws-could-let.html); [Rescana](https://www.rescana.com/post/wasmtime-cranelift-sandbox-escapes-cve-2026-115889-october-2026); [SOCRadar](https://socradar.io/blog/wasmtime-sandbox-escape-cranelift/)
- CVE-2026-115392 is a wasmtime-wasi filesystem sandbox escape. cap-std mishandles symlinks with trailing slashes on macOS and on Linux kernels without `openat2`. Fixed in 24.0.13, 36.0.14, 46.0.3 and 47.0.4+ — [OpenCVE](https://app.opencve.io/cve/CVE-2026-115392); [OSV](https://osv.dev/vulnerability/CVE-2026-115392)
- Resource limits (fuel, epoch interruption, `ResourceLimiter`, pooling allocator): no 2026-specific source was retrieved in this pass. The April advisory confirms the pooling allocator is in active use and had a cross-instance leak (CVE-2026-34988) — [Bytecode Alliance advisories](https://bytecodealliance.org/articles/wasmtime-security-advisories)

### Inferences
- Fyx should pin to a Wasmtime line that receives security backports, such as the 36.x LTS-like line that got 36.0.7, 36.0.14 and 36.0.19, or track latest. It also needs a fast integrator path for Wasmtime bumps, because advisories now arrive in batches.
- On Windows x86_64 with Cranelift (Fyx's primary dev target), the April Criticals did not apply. The October all-architecture Cranelift bug did. Do not ship Winch.
- Defence-in-depth stays necessary even with Wasm. Keep host-side capability checks in WIT host implementations, and never give components raw WASI filesystem preopens beyond what is needed. The wasmtime-wasi symlink escape shows preopens are a weak point.
- WASI 0.3 async is new (June 2026). If `fyx:ui` WIT uses p2-style polling today, migrating to p3 async is plausible but not urgent. Wasmtime 46+ is required.

### Gaps
- Fuel versus epoch-interruption guidance for interactive UI hosts, and their 2026 overhead, was not researched.
- Whether Zed upstream has bumped past the vulnerable Wasmtime versions is not checked here.

## 7. Distribution features users expect (install, update, pinning, rollback, offline, private registries, trust signals)

### Takeaway
Retrieved evidence supports a short list. The platform needs auto-update paired with a remote block list that uninstalls bad versions. It needs a way to force a clean version, because unpublishing alone leaves infected installs (AsyncAPI). Enterprise allowlists are standard: VS Code `AllowedExtensions` and Raycast Teams/Enterprise. Verified-publisher badges exist but are weak signals. Ratings, extension packs/profiles, settings sync and telemetry opt-out were not researched in depth in this pass.

### Cited Findings
- Auto-update plus block list: VS Code auto-uninstalls block-listed extensions — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security)
- Unpublishing without a higher clean version left malicious AsyncAPI installs active for about a month (Nov 26 to Dec 24, 2025) — [Wiz](https://www.wiz.io/blog/snipping-the-long-tail-of-shai-hulud-2-0)
- Enterprise allowlists: VS Code `extensions.allowed` / `AllowedExtensions` policy — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security); [Socket glossary](https://socket.dev/glossary/ide-extension-security). Raycast Teams/Enterprise also has an extensions allow-list — [Raycast allow-list](https://manual.raycast.com/enterprise/extensions-allow-list.md)
- Trust signals: the verified-publisher badge requires domain ownership plus 6 months — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security). 7% of harmful-tagged extensions were nonetheless verified — [arXiv 2411.07479](https://arxiv.org/html/2411.07479v1)
- Ranking signals are gameable: download counts are inflated by bots (fake Solidity with about 2M "downloads"; Eclipse on GlassWorm), and "recently updated" boosted a fake above the real extension — [SC World](https://www.scworld.com/news/fake-visual-studio-code-extension-for-cursor-led-to-500k-theft); [Kaspersky](https://www.kaspersky.com/about/press-releases/kaspersky-uncovers-500k-crypto-heist-through-malicious-packages-targeting-cursor-developers); [Eclipse blog](https://blogs.eclipse.org/post/mika%C3%ABl-barbero/open-vsx-security-update-october-2025)
- Transitive packs are an attack vector: `extensionPack` and `extensionDependencies` were abused by GlassWorm (Mar 2026) — [Socket](https://socket.dev/blog/open-vsx-transitive-glassworm-campaign)
- CLI and offline installs: in VS Code, extensions installed via the CLI are not auto-trusted for publisher trust. Offline/.vsix installs exist, but this was not further researched — [VS Code docs](https://code.visualstudio.com/docs/configure/extensions/extension-runtime-security)
- Publishing hygiene: trusted publishing (OIDC) instead of long-lived tokens — [Alpha-Omega](https://alpha-omega.dev/blog/fewer-keys-under-the-doormat-why-trusted-publishing-matters-for-open-vsx/). Recognisable token prefixes for secret scanning and short token lifetimes — [Eclipse blog](https://blogs.eclipse.org/post/mika%C3%ABl-barbero/open-vsx-security-update-october-2025)
- Per-version automated scanning, with delisting of extensions whose latest version fails, is the Obsidian 2026 model — [Obsidian blog](https://obsidian.md/blog/future-of-plugins/)

### Inferences
Suggested must-have set for Fyx, derived from the incidents in section 1:
1. **Installed-extensions folder and install action, with a manifest-declared capability list shown before install.** High-risk capabilities need explicit consent, and the update path must diff capabilities.
2. **Signed artifacts** (registry signature plus a publisher key) verified at install and at load. Hot reload must re-verify.
3. **Remote block list or kill switch** that disables already-installed versions, plus version pinning and rollback to the previous version. This covers the "no higher version" case.
4. **Staged or delayed auto-update.** Several malicious versions were live for only minutes, so a short quarantine window (hours) before auto-update would have blunted Nx Console and AsyncAPI. This is an inference; no source evaluates delay windows.
5. **Offline/sideload install.** Mark the result as unverified, and do not auto-trust it, as VS Code does for CLI installs.
6. **Enterprise allowlist policy** by extension ID or publisher, plus a private-registry URL setting.
7. **Registry side:** source-PR review (Zed and Raycast model) or per-version automated scanning (Obsidian, VS Code, Open VSX), secret scanning, name-squat protection, trusted publishing (OIDC) and short-lived tokens.
8. For the VS Code sidecar: Open VSX only, with a local cache, a block-list overlay and OS-level isolation beyond Node `--permission`.

### Gaps
- No evidence was gathered in this pass on ratings and reviews, extension packs and profiles, settings sync of extension lists, telemetry opt-out norms, or user surveys of expected distribution features.
- No source evaluates the effectiveness of an update delay or quarantine window.
- No source was retrieved on Figma's or Deno's distribution and permission UX.
