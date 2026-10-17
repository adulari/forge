# Feature: Forgeline theme, glyphs and capability detection

The Forge TUI draws with one visual identity, **Forgeline**: a calm, line-oriented look on the
terminal's own background with one signature, the heat rail (ember while Forge works, cooling to a
neutral line when a turn settles). This page documents the foundation every surface builds on:
colour tokens, capability detection, glyph sets, role styles, the remapping backend and the rules
for writing a renderer. The design lives in `docs/design/`'s spec history; the code lives in
`crates/forge-tui/src/theme/`.

## What you can configure

| Setting | Values | Env override | Default |
|---|---|---|---|
| `[tui] theme` | `auto`, `dark`, `light` | `FORGE_THEME` | `auto` (from `COLORFGBG`, else dark) |
| `[tui] color` | `auto`, `truecolor`, `256`, `16`, `none` | `FORGE_COLOR` | `auto` |
| `[tui] glyphs` | `auto`, `unicode`, `ascii` | `FORGE_GLYPHS` | `auto` |
| `[tui] reduce_motion` | bool | `FORGE_REDUCE_MOTION` | `false` (always on in mono) |

`forge config` shows them under Interface. An environment variable beats the config value, which
beats detection.

## Detection (resolved once, before the first draw)

`theme::init(&TuiConfig)` reads the environment and config and stores an immutable `Theme { tier,
mode, glyph_tier, motion, palette }`. The pure function `detect::resolve(&env_map, &config)` holds
every rule, so the matrix is tested without touching the process environment.

- **Colour tier.** `FORGE_COLOR`, then config. `NO_COLOR` (non-empty) means mono. `TERM=dumb` means
  mono. `COLORTERM=truecolor|24bit` means truecolor. `TERM_PROGRAM` of iTerm.app, WezTerm, vscode or
  ghostty means truecolor, Apple_Terminal means 256. `TERM` containing `256color` means 256;
  `linux`, `xterm`, `screen`, `vt100`, `vt220`, `ansi`, `cygwin` mean 16 colours; anything else
  means 256.
- **Mode.** `FORGE_THEME`, then config, then `COLORFGBG` (last field 0-6 or 8 dark, 7 or 9+ light),
  else dark. The OSC 11 background query is not implemented (it was optional).
- **Glyphs.** `FORGE_GLYPHS`, then config. ASCII for `TERM` of `linux`, `vt100`, `vt220`, `dumb`,
  for a locale (first set of `LC_ALL`, `LC_CTYPE`, `LANG`) that is not UTF-8, and for CJK locales
  where ambiguous-width glyphs are two cells wide. Otherwise Unicode.
- **Motion.** Reduced by `FORGE_REDUCE_MOTION`, config, or the mono tier. Reduced motion turns off
  the spinner glyph cycle (a static `…` stands in, the elapsed counter carries liveness), spinner
  colour cycling, rail cooling, flashes and the WHITE-HOT effort animation.

## Tokens

Every role has an explicit value per tier and mode (`theme/tokens.rs`); nothing is rounded
automatically. 256-colour values are hand-picked xterm indices; 16-colour values are ANSI names so
the user's terminal theme stays in charge; mono keeps only bold, dim and italic.

| token | role | truecolor dark | 256 dark | 16 dark |
|---|---|---|---|---|
| `fg` | body text | #D8DCE8 | 253 | default |
| `fg_muted` | descriptions, outcomes, notices | #9096AA | 246 | default + dim |
| `fg_faint` | hint labels, footers, line numbers | #626779 | 242 | dark gray |
| `line` | unfocused borders and rules | #3A3C4A | 238 | dark gray |
| `brand` | ember: brand mark, live rail, caret, spinner | #FF8A30 | 208 | yellow |
| `brand_hot` | streaming caret, spinner peak | #FFC247 | 214 | light yellow |
| `user` | user rail, paths, info | #6FB1FF | 75 | blue |
| `ok` / `warn` / `err` | success, waiting, failure | #5CD07A / #E8B84A / #F26464 | 78 / 179 / 203 | green / light yellow + bold / red |
| `surface`, `surface_raised` | code band, modal fill, selected row | #18181E, #25252E | 234, 236 | none |
| `diff_*_bg`, `diff_*_word` | diff row and word tints | see `tokens.rs` | | none (bold fg) |
| `cat1..cat6`, `heat0..heat4` | categorical series; ember heat ramp | see `tokens.rs` | | |

The light columns, the 16-colour light column and the mono column are in `tokens.rs`. A unit test
checks every token against the contrast floor on a dark (#101014) and light (#FFFFFF) background:
`fg`, `fg_muted`, `brand`, `user`, `ok`, `warn`, `err` at 4.5:1 or better, `fg_faint` at 3:1.
Backgrounds are never painted across a region: `surface*` and the diff tints exist only in
truecolor and 256 and only for code bands, diff rows, selected rows and modal fills.

## Glyphs

One meaning per glyph, each with an ASCII twin (`theme/glyphs.rs`): brand `✦`/`*`, user rail
`▎`/`|`, assistant rail `▏`/`:`, decision rail `┃`/`#`, prompt and caret `❯`/`>`, disclosure
`▸ ▾`/`+ -`, spinner braille/`| / - \`, awaiting `◇`/`o`, success `✓`/`+`, failure `✗`/`x`,
warning `▲`/`!`, info `·`/`-`, group `◆`/`*`, live dot `●`/`*`, task `○ ◉ ✓`/`o @ x`, gauge
`▰ ▱`/`# -`, branch `⑂`/`git:`, goal `◎`, loop `↻`. Retired: `⚒ ◈ ⚙ ◍ ✎ ※ ⧖ ◷ ⚡ ⇥ ⎇ ⛓ ❓ ⚠ ✖ ✕ ⤢`.
`glyphs::normalise(text)` re-glyphs text that arrives pre-glyphed (command notes, the headless
presenter) and folds the rest to ASCII in the ASCII tier.

## The remapping backend

`theme::backend::RemapBackend` wraps the crossterm backend that `driver.rs` and `tui.rs` build and
remaps every cell colour into the active tier at draw time: RGB to the nearest xterm-256 colour
(CIE76 distance over the cube and grey ramp) in the 256 tier, to the nearest of the 16 ANSI colours
in the 16-colour tier, and to the default colour in mono, always keeping modifiers. Tokens already
hold tier-correct values, so for them it is a no-op. It exists for third-party colours: syntect
highlighting and external statusline SGR (`ansi_line.rs`). Raw ANSI printed outside ratatui (the
startup banner, startup errors) goes through `theme::paint` / `theme::render_error`, which honour
the tier.

## Writing a renderer

- Never write `Color::Rgb`, `Color::Indexed` or a named colour. Ask for a role: `styles::muted()`,
  `styles::brand()`, `styles::rail(kind)`, `styles::notice(kind)`, `styles::temper(label)`,
  `styles::effort(level, tick)`, `styles::gauge(frac)`, `styles::selected_row()`,
  `styles::pill(role)`, `styles::cursor()`, `styles::hint_row(&[(key, label)])`, or the token itself
  through `theme::t().palette`. A unit test walks `src/` and fails on any colour literal outside
  `theme/` and `ansi_line.rs`.
- Draw glyphs from `theme::glyphs::g()` (`g().ok`, `g().spin(tick)`, `g().bar(frac, cells)`,
  `g().border`), never from a literal, so the ASCII tier works. Scrollback lines bake their glyphs
  and colours in when the line is built; the theme never changes at runtime, so that is safe.
- Tests assert text and modifier bits. Scope a tier with `theme::scoped(theme, || ...)`:
  `Theme::new(ColorTier::Mono, Mode::Dark, GlyphTier::Ascii, Motion::Reduced)`.
- Modals take `surface::render_panel` (opaque, neutral border, tone only on the title) and
  `surface::dim_backdrop` to push the chat behind them to faint.

## Verifying by eye

Drive `forge chat --mock` through a private tmux socket (`tmux -L <label>`) with temporary
`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `HOME` and `FORGE_DB`, at 80x24 and 160x48, fullscreen and
`--inline`, under `TERM=xterm-256color`, `TERM=xterm COLORTERM=`, `NO_COLOR=1`,
`FORGE_THEME=light`, `FORGE_GLYPHS=ascii` and `TERM=dumb`.
