# os-int UX Revamp v2 — "Evidence Cockpit"

> **Thesis: os-int is a keyboard-first analyst cockpit where every fact shows its source and nothing gets written without a human's say-so.**
> One layout. One palette. One keymap. Three panes: Rail, Board, and an Inspector that's always there. AI suggestions don't belong in the evidence. They go to a separate Review lane and stay there until you accept them, deliberately.

Plan only. No code changes, PRs or commits. This replaces `UX_REVAMP_PLAN.md`. The problem inventory there (25 items, with file references) still applies, and the main points are summarised in §1.

---

## 1. Why the current UI fails (the short version)

- **It lists everything and ranks nothing.** The nav has 14 equal items (`App::nav_items`, `src/app/mod.rs`), the number keys skip four of them (`src/tui/keymap.rs`), and every view is a flat padded string (`src/tui/screens.rs`, `widgets::list_from`).
- **Accepting AI output is too easy.** Enter, `a` or `y` accepts a proposal (`map_proposal_key`). Elsewhere, Enter just means "open".
- **Provenance is decoration, not structure.** It shows up as a small coloured `[COL]`, and it disappears in two of the three themes (`theme.rs`, `widgets::provenance_span`).
- **The UI looks unfinished.** Headers are padded by byte count (`draw_header`), the hint bar never changes, help is a 70-line hand-maintained text wall (`HELP_TEXT`), and empty states are fake list rows.

## 2. Design commitments (non-negotiable)

1. **Three panes, always.** Rail | Board | Inspector. No layout switching, no floating windows, and no pane hides itself at 96 columns.
2. **Source shown with every fact.** Every row carries a provenance glyph and a source count. The Inspector always shows the full source chain for the selected item.
3. **AI work goes in a separate lane.** Proposals live only in **Review**. They never appear in evidence views until accepted, and once accepted they keep the ◆ glyph permanently.
4. **Writing data takes two keys.** `A` (Shift) selects, then `y` confirms. Enter never writes anything, anywhere.
5. **One key, one meaning, everywhere.** Nothing changes meaning between views.
6. **Glyph + word + colour.** Colour reinforces meaning and never carries it alone.
7. **Passive, and shown as passive.** The UI has no control that could turn on an active tool. Refused tools are listed as refused, with the reason.

## 3. The cuts

| Cut | Why | Where it goes |
|---|---|---|
| **The flat 14-item nav column** | Too many equal choices. | Replaced by a 6-item Rail. |
| **Notes view** | Notes belong to a case or page, not a separate destination. | Inspector tab "Notes" plus `e` to edit in `$EDITOR`. |
| **Artifacts view** | Rarely visited. | Palette `:artifacts` (a modal list) and Export. |
| **Audit view** | Compliance data, not part of daily investigation work. | Inspector "History" tab per object, plus palette `:audit` for the full log. |
| **Sources view** | Sources are context, not a destination. | Inspector "Sources" tab plus search `src:`. |
| **Pages as a separate view** | "Pages" is internal jargon. | Merged into **Targets**, shown on Case. |
| **Agent as a separate view** | Conversation history is secondary. Proposals are what matter. | The transcript becomes the Inspector for an agent run inside Review. |
| **Tools and Settings views** | Configuration doesn't belong on the Rail. | Palette `:tools` and `:settings` modals. |
| **Timeline as a separate view** | Time is a lens on the evidence, not a separate place. | Board toggle `t` on Evidence and Graph. |
| **Proposals modal and Resolution-suggestions modal** | Too cramped for reviewing what will be written. | Merged into the **Review** view. |
| **Themes `default` and `high-contrast`** | Two near-duplicates of one dark theme. | Exactly five built-in themes: **Cockpit Dark** (default), **Cockpit Light**, **Mono**, and two signature themes, **Phosphor** and **Amber** (§5). No user theme files. |
| **Keys `p`, `v`, `r`, `i`, `b`, number keys `6`–`0`, `[`/`]`, view-specific `f`/`x`/`l`** | Collisions and too many letters. | See §6. Ingest, playbooks and the rest go to the palette. |
| **`HELP_TEXT` constant** | It drifts from the real keymap. | Help is generated from the command registry. |
| **`ui.v2` feature flag and dual-path rollout** | **Cut.** See §9. | — |
| **Local web UI** | **Cut.** See §9. | `serve` remains a JSON API. |

## 4. Layout: Rail · Board · Inspector

```
┌ header (1 row) ────────────────────────────────────────────────────────────┐
│ Rail  │ Board (flex)                                │ Inspector (fixed 44)  │
│  14c  │                                             │                       │
│       │                                             │                       │
├───────┴─────────────────────────────────────────────┴───────────────────────┤
│ context bar: keys for the focused pane — generated from the registry        │
└─────────────────────────────────────────────────────────────────────────────┘
```

- **Rail (14 cols), 6 destinations, keys 1–6:** `1 Case` `2 Entities` `3 Evidence` `4 Graph` `5 Review` `6 Jobs`. That's all of them.
- **Inspector is fixed at 44 cols.** Below 110 cols it becomes a bottom drawer (40% height). It is never hidden. `z` zooms the Board temporarily, and any key returns.
- **Minimum 80×24.** Below 80 cols the Rail collapses to digits (`1`–`6`) with icons.
- **Header:** `os-int ▸ <case> ▸ <view>  TLP:AMBER          ◆ 3 to review  ⠋ 2 jobs  agent ● codex`
- **No box around unfocused panes.** Panes are separated by a single `│` rule. The focused pane gets a ▌ accent bar on its title and a bold title.

## 5. Visual system: "Cockpit"

### Colour system

**The rule:** a theme is a fixed set of **15 roles**. Code asks for roles (`theme.ai`), never for raw colours. CI fails on any `Color::` outside `src/tui/theme.rs`. Colour *reinforces* the glyphs (`● ◇ ✎ ◆ ✓ ✗ ⊘`) and never replaces them, and that's what makes the colour-blind numbers below acceptable.

#### Roles

| Role | Meaning |
|---|---|
| `bg` | assumed terminal background. We **don't paint it** in Dark/Light; Phosphor and Amber do paint it |
| `panel` | selected row, chips, the Review confirm bar |
| `fg` / `muted` / `rule` | body text, metadata, separators |
| `accent` | focus bar ▌, key hints, links. The only "look here" colour |
| `ok` / `warn` / `err` | job outcome. `err` also marks refusals |
| `col` / `ext` / `anl` / `ai` | provenance: ● ◇ ✎ ◆ |
| `heat0` → `heat3` | gradient stops for progress bars and graph-edge heat (below) |

#### Cockpit Dark (default)

Contrast ratios are WCAG 2.x, computed against `bg` and against `panel` (the selected row). The AA threshold for body text is 4.5:1.

| Role | Hex | vs bg `#14171C` | vs panel `#1B1F26` |
|---|---|---|---|
| fg | `#D8DEE6` | 13.3 | 12.2 |
| muted | `#8B95A1` | 5.9 | 5.4 |
| rule | `#2C323B` | (non-text) | — |
| accent | `#4FB3D9` | 7.5 | 6.9 |
| ok | `#8FD694` | 10.4 | 9.6 |
| warn | `#E5B567` | 9.5 | 8.8 |
| err | `#E0607E` | 5.3 | 4.8 |
| col ● | `#56C2A6` | 8.3 | 7.6 |
| ext ◇ | `#A9B1D6` | 8.5 | 7.8 |
| anl ✎ | `#D8DEE6` **bold** | 13.3 | 12.2 |
| ai ◆ | `#C792EA` *italic* | 7.5 | 6.9 |
| heat0→3 | `#2C4A5A` → `#4FB3D9` → `#E5B567` → `#E0607E` | (fills, non-text) | |

#### Cockpit Light

| Role | Hex | vs bg `#F7F7F5` | vs panel `#EEEEEA` |
|---|---|---|---|
| fg | `#1D232B` | 14.7 | 13.6 |
| muted | `#5C6670` | 5.5 | 5.0 |
| rule | `#C9CDD2` | (non-text) | — |
| accent | `#0B6E99` | 5.3 | 4.9 |
| ok | `#1E5B2A` | 7.6 | 7.0 |
| warn | `#8A5A00` | 5.5 | 5.1 |
| err | `#C2185B` | 5.5 | 5.0 |
| col ● | `#17705A` | 5.6 | 5.2 |
| ext ◇ | `#4A5394` | 6.6 | 6.1 |
| anl ✎ | `#1D232B` **bold** | 14.7 | 13.6 |
| ai ◆ | `#7B3FA0` *italic* | 6.4 | 5.9 |
| heat0→3 | `#D6E4EC` → `#0B6E99` → `#8A5A00` → `#C2185B` | (fills) | |

Changes from the earlier draft: Dark `ok` `#7BC47F`→`#8FD694` and `err` `#E06C75`→`#E0607E`. Under deuteranopia, ok and err had collapsed to ΔE 9; they're now ΔE 19. Light `ok` became `#1E5B2A` and `col` became `#17705A` (both darker), so they pass 4.5:1 on `panel` and keep ok/err at ΔE ≥ 15 under every simulation, and Light `err` moved toward crimson `#C2185B` so it no longer merges with `warn` under deuteranopia (ΔE 5 → 35).

#### Mono (forced by `NO_COLOR`; also the screen-reader default)

| Role | Attribute |
|---|---|
| fg | default |
| muted | default (**not** DIM, which is unreadable on many terminals) |
| accent / focus | **bold** + ▌ |
| selection | REVERSED |
| ok / warn / err | default + glyph `✓` / `!` / `✗` |
| col / ext / anl / ai | glyph only; ai adds *italic* where the terminal supports it, otherwise the `*` prefix in ASCII |
| heat | none: progress uses `█▓▒░` density, graph heat uses edge weight `─` `━` `═` |

Contrast is whatever the user's terminal gives. Mono never relies on colour, so it's the reference rendering for tests.

#### Signature themes (opt-in, for people who live in the terminal)

These are deliberately monochromatic. In both of them, provenance is **glyph-only by design**, and the colour roles below are brightness steps within one hue. Error is the one break from the hue, so failures still jump out.

**Phosphor:** a green CRT look. It paints `bg`. In full motion it can show a subtle scanline (alternate `panel` rows at 3% luminance), but that's off by default and never used for data.

| Role | Hex | vs bg `#050A06` | vs panel `#0C1A10` |
|---|---|---|---|
| fg | `#7CFF9B` | 15.8 | 14.2 |
| muted | `#3FAE5F` | 7.1 | 6.3 |
| accent | `#C8FFD4` | 17.8 | 16.0 |
| ok | `#7CFF9B` | 15.8 | 14.2 |
| warn | `#E8FF7A` | 18.1 | 16.2 |
| err | `#FF6B5E` | 7.1 | 6.4 |
| col / ext / ai | `#4DE680` / `#9FD9B0` / `#B8FFC9` italic | 12.3 / 12.4 / 17.3 | 11.1 / 11.1 / 15.5 |
| heat0→3 | `#0C3B1A` → `#1F9E44` → `#7CFF9B` → `#E8FF7A` | | |

**Amber:** a monochrome amber terminal look, easier on the eyes in a dark room.

| Role | Hex | vs bg `#0E0900` | vs panel `#1C1300` |
|---|---|---|---|
| fg | `#FFB000` | 10.8 | 10.0 |
| muted | `#B88220` | 5.9 | 5.5 |
| accent | `#FFD580` | 14.3 | 13.2 |
| ok | `#FFC94D` | 13.0 | 12.0 |
| warn | `#FFE9B0` | 16.6 | 15.3 |
| err | `#FF6A3D` | 7.0 | 6.5 |
| col / ext / ai | `#FFA31A` / `#E0B870` / `#FFE0A3` italic | 9.9 / 10.6 / 15.5 | 9.2 / 9.9 / 14.4 |
| heat0→3 | `#3A2600` → `#B88220` → `#FFB000` → `#FF6A3D` | | |

There's no Phosphor/Amber light variant. Both are dark-only, and `auto` never selects them.

#### Colour-blind check

Method: Machado-2009 simulation (severity 1.0), then CIELAB ΔE between every pair of semantic roles (`ok warn err col ext ai accent`). ΔE < 12 counts as "confusable at a glance".

| Theme | Deutan | Protan | Tritan | Pairs still close | Why it's acceptable |
|---|---|---|---|---|---|
| Dark | min 5 (ai/accent) | min 4 (ext/accent) | min 7 (col/accent) | accent vs a provenance colour | accent never sits on a provenance glyph; it's used on the focus bar and key hints only |
| Light | min 4 (ext/ai) | min 10 (ext/ai) | min 8 (col/accent) | ext/ai; ok/col (ΔE 10); accent vs col | ◇ vs ◆ glyphs + ai italic; ok only appears in Jobs, col only in provenance columns; accent never sits on a provenance glyph |

**Critical pairs, worst case across deutan/protan/tritan:** ok/err Dark 19, Light 15; warn/err Dark 37, Light 35. These are the pairs that must never be confused, and they no longer can be.
CI recomputes this table from `theme.rs` (a small test using the same matrices) and fails if a critical pair drops below 15.

#### Fallbacks: truecolour → 256 → 16

**Detection (decided):** truecolour if `COLORTERM` is `truecolor` or `24bit`, else 256 if `TERM` contains `256color`, else 16. `NO_COLOR` (any value) → Mono. `ui.color = auto | truecolor | 256 | 16 | none` overrides. No terminal queries, no escape-sequence probing at startup: it's fragile over SSH/tmux and slows boot.

| Role | Dark 256 (idx → hex, CR vs 234) | Dark 16 | Light 256 (idx → hex, CR vs 255) | Light 16 |
|---|---|---|---|---|
| bg | 234 `#1C1C1C` | Black (unpainted) | 255 `#EEEEEE` | White (unpainted) |
| panel | 235 `#262626` | Black + REVERSED sel | 254 `#E4E4E4` | REVERSED |
| fg | 253 `#DADADA` (12.2) | White | 235 `#262626` (13.0) | Black |
| muted | 246 `#949494` (5.6) | Gray | 241 `#626262` (5.3) | DarkGray |
| accent | 74 `#5FAFD7` (7.0) | Cyan | 24 `#005F87` (6.1) | Blue |
| ok | 114 `#87D787` (9.8) | Green | 22 `#005F00` (6.9) *(override: nearest match was grey 238)* | Green |
| warn | 179 `#D7AF5F` (8.3) | Yellow | 94 `#875F00` (4.9) | Yellow → **bold** |
| err | 168 `#D75F87` (4.8) | LightRed | 125 `#AF005F` (6.1) | Red |
| col ● | 73 `#5FAFAF` (6.7) | Cyan | 23 `#005F5F` (6.5) | Cyan |
| ext ◇ | 146 `#AFAFD7` (8.1) | Blue | 60 `#5F5F87` (5.2) | Blue |
| ai ◆ | 176 `#D787D7` (6.8) | Magenta + italic | 90 `#870087` (7.6) *(override: nearest match was 97, 4.2)* | Magenta + italic |

At 16 colours, accent and col share Cyan. That's acceptable because glyph shape decides. Phosphor and Amber at 256 map to their nearest indices; at 16 they **degrade to Mono with a status hint** ("phosphor needs 256 colours"). Pretending green is phosphor would look bad.

#### Gradients (motion and data only, never decoration)

Gradients interpolate in **OKLab** between the four `heat` stops and are quantised to the active colour depth: truecolour gets 16 steps; 256 gets the nearest index per step, deduplicated; 16 colours gets none, and the gradient becomes glyph density (`░▒▓█`).

- **Progress bars:** the filled portion runs heat0→heat1 left to right; the leading cell is heat2 while active. On completion the whole bar flashes `ok` for one frame and then settles to `muted`. On failure the bar turns `err` from the failure point onward.
- **Graph edge heat:** encodes how many sources back an edge (1 → heat0, 2–3 → heat1, 4–6 → heat2, 7+ → heat3) **and** stroke weight `─ ━ ═` for Mono/16. Heat means corroboration, never risk or severity: the UI makes no threat judgments it didn't collect.
- **Scan/radar sweep** (§8): the trail decays heat2 → heat1 → heat0 → `rule`.

#### Theme preview command (decided)

`os-int theme preview [name|--all]` prints, without opening the TUI:
1. a swatch row per role with hex, 256 index, 16 name and live contrast vs bg/panel;
2. a rendered 80×12 sample (a Rail, a provenance table and a Review confirm bar) in that theme;
3. the colour-blind ΔE table;
4. the detected colour depth and why (`COLORTERM=truecolor`).

`os-int theme preview --all` stacks all five themes. In the TUI, palette `:theme` cycles themes live, `y` saves to config, `Esc` reverts, and `Enter` previews. This command is also the snapshot fixture for UI tests.


### Glyphs (Unicode; no Nerd Fonts; `ui.ascii = true` gives the fallback)

| Meaning | Glyph | ASCII |
|---|---|---|
| Collected | `●` | `C` |
| External/imported | `◇` | `X` |
| Analyst | `✎` | `A` |
| AI (accepted) | `◆` | `*` |
| AI (pending) | `◆?` | `*?` |
| Running | `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` | `...` |
| Done / failed / refused | `✓` `✗` `⊘` | `ok` `x` `no` |
| Focus bar | `▌` | `>` |
| Edge | `─▶` `◀─` | `->` `<-` |
| Entity kinds | `DOM IP URL EML USR REP ASN FIL ORG` (3-char tags, fixed-width column) | same |

### Typography & spacing
Bold appears only on titles, the focused pane and the selected row. Tables have one header row in `muted`, 2-space gutters, right-aligned numbers, and relative times (`3h`) in lists. The Inspector shows absolute ISO-8601 times labelled with their timezone.

## 6. The keymap (the only one)

| Key | Action | Notes |
|---|---|---|
| `1`–`6` | Rail destinations | match the visible order, always |
| `j` `k` / `↑` `↓` | move | |
| `h` `l` / `←` `→` / `Tab` | move focus Rail ◀▶ Board ◀▶ Inspector | |
| `Enter` | open / drill in | **never writes** |
| `Esc` | back (history stack) / close | |
| `/` | filter the current table inline (`kind:ip src:ct prov:ai`) | |
| `Ctrl+K` | **command palette** (also the only home for ingest, playbook, export, tools, settings, audit, artifacts, marking, team) | `:` is an alias |
| `?` | help for the current pane, generated from the registry | |
| `c` | collect on the selected target (passive collectors only) | was `r` |
| `s` | Search Around from the selection | |
| `g` | open the selection in Graph | |
| `t` | toggle the time lens (Evidence/Graph) | replaces the Timeline view |
| `n` | new (target, finding, note — whichever fits the Board) | |
| `e` | edit the note in `$EDITOR` | |
| `a` | ask the agent about the selection → results land in Review | |
| `Space` | mark | |
| `m` | propose a merge of the marked rows → Review | |
| `+` `-` | hops (Graph) / time bucket (time lens) | |
| `A` | **select for accept** in Review (shows a confirm bar) | |
| `y` | confirm the pending accept | only valid right after `A` |
| `x` | reject (Review) / clear filter (elsewhere) | always means "remove/negate" |
| `z` | zoom the Board | |
| `q` | quit (confirms if jobs are running) | |

Deleted outright: `p` (palette moves to Ctrl+K), `v` (Enter opens), `r`, `i`, `b`, `o`/`d` on Welcome, `[`, `]`, `f`, `l`, `u` (unresolve moves to the palette, with confirm), `7`–`0`. A one-release `ui.legacy_keys` setting is **not** provided. The palette shows each command's new key, which is how people migrate.

## 7. Key screens

### 7.1 Case (home, `1`)
```
 os-int ▸ acme-phish ▸ Case   TLP:AMBER                  ◆ 3 to review  ⠋ 2 jobs  agent ● codex
▌Case    │ TARGETS                              last collect     │ ▌Inspector · acme-login.com
 Entities│ DOM acme-login.com          ● 14     12m ✓            │ DOM acme-login.com
 Evidence│ IP  203.0.113.7             ● 3      ⠋ running        │ ● 14 facts from 6 sources
 Graph   │ EML billing@acme-login.com  —        never  c collect │
 Review 3│                                                       │ SOURCES
 Jobs  ⠋2│ AT A GLANCE                                           │  S1 ● dns     2026-10-14 15:31 MDT
         │  41 entities   ● 32  ◇ 4  ✎ 3  ◆ 2                    │  S2 ● rdap    2026-10-14 15:31 MDT
         │  18 findings   ● 11  ✎ 5  ◆ 2                         │  S3 ● ct      2026-10-14 15:32 MDT
         │                                                       │  S4 ● tls     2026-10-14 15:32 MDT
         │ NEXT                                                  │
         │  ◆ 3 agent proposals waiting          5 review        │ NOTES  e edit
         │  ! 1 target never collected           c collect       │  Registered 3 days before
         │                                                       │  the campaign. ✎
 c collect  n new target  a ask agent  s search around  Ctrl+K commands  ? help
```

### 7.2 Entities (`2`) with the Inspector
```
 os-int ▸ acme-phish ▸ Entities  TLP:AMBER               ◆ 3 to review  ✓ jobs  agent ● codex
 Case    │ /kind:dom                                 41 → 9      │ ▌Inspector · login.acme-login.com
▌Entities│ KIND VALUE                     PROV  SRC  SEEN        │ DOM login.acme-login.com
 Evidence│ DOM acme-login.com             ●     6    12m         │
 Graph   │▌DOM login.acme-login.com       ●     2    12m         │ PROPERTIES
 Review 3│ DOM mail.acme-login.com        ●◆    2    1h          │  resolves_to  203.0.113.7    ● S1
 Jobs    │ DOM acme-l0gin.com             ◇     1    2d          │  cert_issuer  Let's Encrypt  ● S3
         │                                                       │  first_seen   2026-10-11     ● S3
         │                                                       │  role         phishing host  ◆ R12
         │                                                       │
         │                                                       │ LINKS  (3)
         │                                                       │  ─▶ resolves_to  IP 203.0.113.7
         │                                                       │  ◀─ subdomain_of DOM acme-login.com
         │                                                       │ TABS  Props · Sources · Notes · History
 Enter open  s search around  g graph  t time  Space mark  m propose merge  / filter
```

### 7.3 Review (`5`): the lane where AI and automation proposals wait
```
 os-int ▸ acme-phish ▸ Review  TLP:AMBER                  ◆ 3 to review  ✓ jobs  agent ● codex
 Case    │ AI PROPOSES ONLY · nothing is written until you accept│ ▌Will write
 Entities│                                                       │ + Finding  ◆ AI
 Evidence│ ◆ agent run R12 · codex · 14:58                       │   "acme-login.com is a phishing
 Graph   │  ▌◆? Finding: acme-login.com is phishing infra        │    host impersonating acme.com"
▌Review 3│   ◆? Link: mail.acme-login.com ─▶ uses_mx  203.0.113.7│   severity  medium
 Jobs    │ ⚙ playbook domain-infra · 15:10                       │   provenance agent (kept forever)
         │   ◆? Entity: ASN 64500                                │
         │ ≈ resolution suggestion                               │ CITED EVIDENCE
         │   ✎? Merge acme-l0gin.com + acme-login.com  score .82 │  S1 ● dns   A 203.0.113.7
         │                                                       │  S3 ● ct    crt.sh id 9912…
         │                                                       │ RATIONALE ◆
         │                                                       │ ┊ Registered 3 days before the
         │                                                       │ ┊ campaign; cert SAN mimics …
├─────────────────────────────────────────────────────────────────────────────────────────────┤
│ Accept 1 AI finding? It will keep ◆ agent provenance.   y confirm   any other key cancels  │
└─────────────────────────────────────────────────────────────────────────────────────────────┘
```
Rules: Enter only expands. `A` arms, `y` writes, anything else disarms. `Space` + `A` batch-arms with a count ("Accept 4 items, 3 AI"). `x` rejects, with an optional reason. Citations that point at missing sources show `✗` and block acceptance.

### 7.4 Graph (`4`)
```
 os-int ▸ acme-phish ▸ Graph · acme-login.com · 2 hops · 12/200 nodes      ◆ 3  ✓  agent ●
 Case    │ KIND   DOM 6  IP 3  EML 2  ASN 1      x clear │ ▌Inspector · 203.0.113.7
 Entities│                                               │ IP 203.0.113.7
 Evidence│  DOM acme-login.com ●                         │ ● 3 facts · 2 sources
▌Graph   │   ├─ subdomain ─▶ DOM login.acme-login.com ●  │
 Review 3│   │                └─ resolves_to ─▶ ▌IP 203.0.113.7 ●
 Jobs    │   ├─ resolves_to ─▶ IP 203.0.113.7 ● (↑ seen) │ asn       64500 ● S2
         │   ├─ mx ─────────▶ DOM mail.acme-login.com ●  │ country   NL    ● S2
         │   └─ cert_san ───▶ DOM acme-l0gin.com ◇       │
         │  +4 more at hop 2 · + expand                  │ LINKS (4)
 Enter focus node  s search around  + - hops  t time  / filter  g recenter
```
**Decision:** the graph is an **edge-labelled tree from the focused node, with cross-links marked `(↑ seen)`**. It's readable at 80 cols, works with screen readers, and the code is cheap. A braille force-directed canvas is **cut**: it looks impressive and is unreadable for real analysis in a terminal.

### 7.5 Empty / loading / error (shared widget)
```
            ◇  No entities yet
            Collect on a target to populate this case.
            c collect · n new target
```
Loading puts skeleton rows `░░░░` in the Board and a spinner tied to the job id. Errors show an inline panel with cause, remedy and `Ctrl+K doctor`, which stays until dismissed with `x`.

### 7.6 Refusals
The palette `:tools` lists tools as `⊘ nmap  active  refused: os-int observes public information only`. There is no toggle and no "enable anyway" option.

## 8. Motion

**Thesis: motion only tells you about state.** Every animation answers one of: *something is working*, *something changed*, *you moved*, or *this needs you*. If an animation answers none of these, it's cut. Nothing loops while idle. A cockpit with nothing running is perfectly still.

### 8.1 Engine (decided)

- **One clock: a 60 ms tick (≈16.7 fps).** It's fast enough for smooth spinners and slide-ins, and slow enough that SSH, tmux and Windows Terminal never fall behind. All durations below are whole ticks. There's no 30/60 fps mode, because a terminal TUI gains nothing from it.
- **The tick only runs while something is animating.** The event loop in `src/tui/mod.rs::event_loop` selects on `crossterm::EventStream`, the job channel and a `tokio::time::interval(60ms)` that's **created when the first animation starts and dropped when the last one finishes**. When idle, the TUI wakes only on input or job events: 0 timer wakeups.
- **Animation state lives in `src/tui/motion.rs`:** an `Animator` holding `Vec<Anim { id, kind, start_tick, frames, easing, region: Rect }>`. Widgets read `animator.frame(id)` while rendering, and they stay pure functions of `(state, frame)`, so snapshot tests can render any frame deterministically (`Animator::freeze_at(tick)`).
- **Redraw only what changed.** ratatui already diffs the back buffer against the front buffer and emits only changed cells. We add a gate: on a tick where only animations advanced, `terminal.draw` is skipped unless some `Anim.region` produced a different frame index. Spinner ticks therefore cost one tiny diff (1–3 cells), not a full-screen repaint. A **full redraw** happens only on resize, theme change or view change.
- **Easing, one curve per job:**
  - `ease-out-cubic` `1-(1-t)^3` for anything entering (toasts, drawers, graph nodes, the confirm bar);
  - `ease-in-quad` `t^2` for anything leaving;
  - `linear` for progress and spinners (never ease a progress number: it lies);
  - `step` for the focus pulse (terminals can't blend, so this is discrete brightness steps).
- **Budget (enforced):**
  - **≤ 2% of one core** while 3 jobs and the agent are running at 120×40 (measured with `perf stat` in a bench test);
  - **≤ 1 ms of render time per tick** (p99), and ≤ 400 bytes written to the tty per animation tick;
  - **0% when idle.**
  - Concurrency cap: at most 1 sweep, 1 toast, 1 transition and any number of 1-cell spinners at once. Extra toasts queue.
- **Settings:** `ui.motion = full | reduced | off`. **`reduced`** keeps state changes but removes travel: no slides or sweeps, spinners become a static `…` that updates once per second, and transitions are instant. **`off`** has no timer at all; every state renders its final frame. `--plain`, linear/screen-reader mode, `NO_COLOR` and `TERM=dumb` all force **`off`**. Reduce-motion is also automatic if the tty write queue backs up (more than 3 ticks behind) — common over slow SSH.

### 8.2 The motion catalogue (only these; adding one needs a row here)

| # | Animation | Means | Frames × tick | Duration | Easing | Reduced | Off |
|---|---|---|---|---|---|---|---|
| M1 | Boot / splash | the app is opening your case | 10 | 600 ms, skippable by any key | ease-out | 1 static frame, 150 ms | none; straight to Case |
| M2 | Job spinner (collector) | a passive collector is running | 10-frame braille loop | 600 ms/cycle | linear | `…` 1 Hz | `running` text |
| M3 | Agent spinner | the agent is thinking | 4-frame ◆ pulse | 960 ms/cycle | step | `◆…` | `thinking` text |
| M4 | Progress bar | known-size work (CT pages, ingest rows, playbook steps) | continuous | follows real progress | linear | updates ≤ 2 Hz | `42%` text |
| M5 | Focus pulse | focus moved to another pane | 3 | 180 ms, once | step | instant | instant |
| M6 | Pane / drawer transition | the Inspector drawer opens (narrow terminals) | 4 | 240 ms | ease-out-cubic in, ease-in-quad out | instant | instant |
| M7 | Graph expand | Search Around / `+` added hops | 2 ticks per hop ring, ≤ 6 | ≤ 360 ms | ease-out-cubic | new nodes appear marked `+`, no reveal | same as reduced |
| M8 | Toast slide-in | a job finished, failed or was refused | 4 in, hold 3 s, 3 out | 240 ms in / 180 ms out | ease-out / ease-in | appears in place, hold 4 s | goes to the message line only |
| M9 | Review confirm bar | `A` armed a write | 3 rise + 2-step pulse on `y` | 180 ms + 120 ms | ease-out-cubic | instant, no pulse | instant |
| M10 | Radar sweep | passive collection is observing a target | 8-frame sweep loop | 960 ms/cycle | linear | static `◎ observing` | `observing` text |

**Cut:** typing effects for agent text (they fake latency), a parallax splash, animated borders, an idle screensaver, and bouncing/elastic easing. None of them convey state.

### 8.3 Frame-by-frame

**M1 Boot / splash (10 frames × 60 ms = 600 ms).** The wordmark resolves in, then the case name. Any key skips it. It's only shown on cold start with a TTY, never on `os-int open` when `ui.motion != full`.
```
f0-1            f2-3              f4-5               f6-7                 f8-9
 .              ░░ ░  ░           ▒▓ ▒▓▒ ▒▓          os-int               os-int
                                                     ──────               ──────
                                                                          ▸ acme-phish  TLP:AMBER
                                                                          passive · ◆ proposes only
```
The rule `──────` draws left to right in heat0→heat1 on f6–7. Frames 8–9 add the case line in `muted` and the operating rules in `accent`. The splash states the rules on every start.

**M2 Collector spinner (braille, 10 frames, 1 cell, 60 ms each).** It sits in the Jobs row, the Rail badge and the header.
```
⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏   →  on done: ✓ (ok, holds)   on fail: ✗ (err, holds)
```

**M3 Agent pulse (4 frames × 4 ticks = 960 ms cycle).** It's slower than the collector spinner on purpose, so "the agent is reasoning" never looks like "a collector is fetching".
```
◇  ◈  ◆  ◈        colour: ai at 40% → 70% → 100% → 70% (truecolour); 16-colour: glyph change only
```

**M4 Progress bar (12 cells, eighth-block precision, linear).**
```
t=0     [            ]   0%   ct · crt.sh page 0/8
t=…     [███▋        ]  31%   ct · crt.sh page 2/8     ▋ = leading cell in heat2
t=…     [█████████▎  ]  78%
done    [████████████] 100% ✓ 1 frame in ok, then muted
fail    [██████▍▒▒▒▒▒]  54% ✗ remainder becomes ▒ in err
```
An unknown total never gets a fake bar. It uses M2 or M10 instead.

**M5 Focus pulse (3 frames, step).** The ▌ bar on the newly focused pane goes bright → normal; the pane you left doesn't animate.
```
f0  ▌Inspector   (accent, bold, reverse ▌)
f1  ▌Inspector   (accent, bold)
f2  ▌Inspector   (accent)          ← resting state
```

**M6 Inspector drawer (narrow terminals, 4 frames = 240 ms, ease-out-cubic → heights 42%, 71%, 90%, 100% of target).**
```
f0 ─────────          f1 ─────────          f2 ─────────          f3 ─────────
   ▌Inspector            ▌Inspector            ▌Inspector            ▌Inspector
                         DOM acme-…            DOM acme-login.com    DOM acme-login.com
                                               ● 14 facts            ● 14 facts · 6 sources
```

**M7 Graph expand (≤ 3 hop rings × 2 ticks).** New nodes reveal ring by ring from the focus. The edges draw first, then the labels. New nodes keep a `+` marker until the next keypress, so you can see what Search Around actually added.
```
t0                         t2                                     t4
DOM acme-login.com ●       DOM acme-login.com ●                   DOM acme-login.com ●
                            ├─ resolves_to ─▶                      ├─ resolves_to ─▶ IP 203.0.113.7 ● +
                            └─ mx ─────────▶                       └─ mx ─────────▶ DOM mail.acme… ● +
```
Edge colour follows corroboration heat (§5). If the expansion would cross the node cap, it doesn't animate: it stops and asks first (existing behaviour).

**M8 Toast (top-right of the Board, 34 cols; slides in from the right edge).** It never steals focus and never covers the Inspector.
```
f0                    │░f1               │ ✓ ct│f2             │ ✓ ct done · 37 c│f3  │ ✓ ct done · 37 certs  S9 │
```
Kinds: `✓ ok` done, `✗ err` failed (holds until `x` or 8 s), and `⊘ err` refused ("active tool refused: nmap"). Refusal toasts always say *why*. Toasts stack at most 3; older ones collapse to `+2`.

**M9 Review confirm bar (rises 3 frames from the bottom, panel bg).**
```
f0 ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁
f1 ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄
f2  Accept 1 AI finding? It keeps ◆ agent provenance.   y confirm · any other key cancels
on y:  bar flashes ok for 2 ticks → "✓ written · ◆ AI · R12" toast (M8) → the row leaves Review with ease-in-quad
on cancel: bar drops in 2 frames; nothing written
```
There's **no** countdown or auto-confirm, ever. The bar waits indefinitely. The animation only makes the armed state obvious.

**M10 Radar sweep (passive collection; 8 frames × 2 ticks = 960 ms cycle; 5×3 cells).** It's shown in the Case targets row and the Jobs detail for the target being observed. It reads as listening, not attacking. There are no crosshairs and no "hit" flashes.
```
f0      f1      f2      f3      f4      f5      f6      f7
 ╭─╮     ╭─╮     ╭─╮     ╭─╮     ╭─╮     ╭─╮     ╭─╮     ╭─╮
 │◎│ ↑   │◎│ ↗   │◎│ →   │◎│ ↘   │◎│ ↓   │◎│ ↙   │◎│ ←   │◎│ ↖
 ╰─╯     ╰─╯     ╰─╯     ╰─╯     ╰─╯     ╰─╯     ╰─╯     ╰─╯
```
The arrow is heat2. The previous two positions stay drawn as heat1 and heat0 (that's the trail). It stops the moment the last passive collector for that target ends, and then shows `✓` or `✗`. There's no sweep for active anything, because there is no active anything.

### 8.4 Testing motion
- Snapshot tests render every animation at **its first frame, middle frame and last frame**, plus `motion=off`.
- There's a bench test for the CPU and bytes-per-tick budget.
- A unit test asserts that the 60 ms interval is dropped when there are no active animations (idle = 0 wakeups).

## 9. The decisions I'm not hedging on

- **No `ui.v2` flag and no dual rendering path.** The repo's cross-phase rule 6 *allows* dark flags but doesn't require them. Running two renderers for one TUI doubles snapshot tests and review work for a single-user local tool. Each phase replaces the old screens in place, with golden snapshots as the safety net. Rollback = `git revert` of one PR.
- **No web UI. Cut it.** The analyst is in a terminal. A browser view would add a second design system, more attack surface on the loopback server, and a tempting place to put an "accept" button outside the confirm flow. `serve` stays a JSON API for integrations. Accepting proposals is TUI/CLI only, permanently.
- **No user-configurable layout.** One layout, tested at 80×24, 120×40 and 200×50.
- **No legacy keymap.** A single migration note goes in `docs/guide.md` and the CHANGELOG.
- **Five themes, no custom theme files.** `ui.theme = auto | dark | light | mono | phosphor | amber`. `auto` (the default) picks dark or light from `COLORFGBG`, and `NO_COLOR` forces Mono. Signature themes are opt-in only; they're never auto-selected.
- **Motion is on by default, but it's never decorative** (§8). `ui.motion = full | reduced | off`. Plain/screen-reader mode forces `off`.
- **The CLI gets the same glyph vocabulary** (`●◇✎◆ ✓✗⊘`). It's coloured only on a TTY, every list command has `--json`, and `collect` ends with a per-collector ✓/✗ table.

## 10. Phases (5 PR gates, builder → reviewer, docs in the same PR)

There's still no `ui.v2` flag. Each phase replaces code in place, with snapshot tests as the safety net.

**P1 · Foundation: roles, themes, registry (M, ~900 LOC)**
Rewrite `theme.rs` as 15 roles for all five themes. Add colour-depth detection (`COLORTERM`/`TERM`/`NO_COLOR`, `ui.color` override), the 256/16 fallback tables and the glyph module (ASCII set too). Add the command registry driving the palette, context bar and generated help, and delete `HELP_TEXT`. Fix the `draw_header` width bug. Add `os-int theme preview [--all]`.
*Done when:* no `Color::` outside `theme.rs` (CI grep); a CI test recomputes WCAG ratios for every text role on bg and panel (≥ 4.5:1 in Dark/Light/Phosphor/Amber) and the critical-pair colour-blind ΔE (≥ 15); `theme preview --all` output is snapshot-tested; Phosphor/Amber degrade to Mono at 16 colours with a hint.

**P2 · Cockpit layout + Review lane (L, ~1.2k LOC). This is the safety phase.**
Build the Rail/Board/Inspector frame, the 6-item Rail and the single keymap. Add the Review view, absorbing the proposals and suggestions modals. Add `A`→`y` arming with a **static** confirm bar (animation comes in P4). Remove the seven old views and move them to Inspector tabs and the palette.
*Done when:* tests prove Enter/`a`/`y`-alone can't accept; accepted items keep `agent` provenance; number keys match the Rail; no key has two meanings (registry test); every removed function is reachable from the palette.

**P3 · Evidence tables, Inspector provenance, states (L, ~1k LOC)**
Use `Table` for Entities/Evidence with the `/` filter grammar. Add Inspector source footnotes and tabs, shared Empty/Loading/Error widgets, the time lens `t`, and Tools refusals with reasons.
*Done when:* every fact row shows a glyph + source count; the source chain is ≤ 1 keypress away; zero "(no …)" pseudo-rows; refused tools show `⊘` with a reason and have no enable path.

**P4 · Motion & signature themes (M, ~800 LOC)**
Add `src/tui/motion.rs` (Animator, 60 ms on-demand tick, a region-gated redraw), animations M1–M10, the gradient engine (OKLab, depth-quantised), and graph-edge corroboration heat. Add `ui.motion = full|reduced|off` and the automatic `off` in plain/`NO_COLOR`/`TERM=dumb`. Add palette `:theme` live cycling, and finalise the Phosphor/Amber visual polish (optional scanline).
*Done when:* idle CPU is 0 timer wakeups (unit test); a bench shows ≤ 2% of one core and ≤ 1 ms p99 render with 3 jobs + the agent at 120×40, and ≤ 400 bytes/tick; snapshot first/mid/last frames for M1–M10 plus `motion=off`; in `reduced`/`off` no frame contains travel (slide or sweep); the Review confirm bar has no timer path that could auto-confirm (code review + test); the splash is skippable with any key and never blocks for more than 600 ms. Implemented in this phase.

**P5 · Graph tree, Case home, CLI parity, a11y sign-off (M, ~800 LOC)**
Build the edge-labelled graph tree using M7 and edge heat, the Case dashboard with NEXT actions and M10 sweeps, CLI glyph/`--json` parity (the CLI respects the theme's 16-colour mapping and never animates when stdout isn't a TTY), and an 80×24 audit.
*Done when:* the demo graph is readable at 80 cols; the hallway test (3 new users: create → collect → accept an AI finding → export, no help screen) passes; all snapshots pass in Mono + ASCII + `motion=off`; the screen-reader pass in plain mode hears no spinner churn.


## 11. Reviewer gate (every PR)
- No active tool can be enabled from any surface.
- No write path without `A`→`y` (or an explicit CLI flag); Enter never writes.
- ◆ stays on accepted AI items; ● is never relabelled.
- The source chain is visible from every fact.
- Mono + ASCII snapshots pass, and the PR includes a `motion=off` snapshot for any animated widget.
- No new animation without an entry in the §8 motion table (purpose, frames, duration, reduced-motion fallback).
- `guide.md`, ARCHITECTURE and the generated help are updated in the same PR.
