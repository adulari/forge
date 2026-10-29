# Rich replies: math, images, path chips, gallery

What the desktop, web and phone chat shows beyond plain prose. The terminal UI keeps tables and
names an image's source instead (`[image: the cart] (docs/cart.png)`).

## In a reply or a prompt

| Written as | Shown as |
|---|---|
| a GFM pipe table | a table with aligned columns that scrolls sideways when it is wider than the pane |
| `$E=mc^2$`, `\(x\)` | typeset inline math |
| `$$ … $$`, `\[ … \]` on their own lines | typeset display math |
| `![alt](docs/shot.png)` on its own line | the project image, at most 320 px tall; click to open the viewer |
| `src/a.ts:12`, `crates/x/lib.rs`, `Cargo.toml`, `` `docs/a.md` `` | a path chip with a dot in the file type's colour; click or Enter opens the Files dock at that line |
| `@src/a.ts` in your own prompt | the same chip |

Prices survive: `$5 and $10` is text, because a closing `$` is never followed by a digit and an
opening one is never followed by a space. A `$$` block that has not closed yet (a reply still
streaming) stays text until the closing line arrives. Bare names only become chips with a known
source or document extension, and product names such as `Node.js` are left alone.

Math is typeset by KaTeX as MathML, so it inherits the surrounding colour and size and ships no
fonts. The library is its own chunk, fetched the first time a reply contains math. Phones show the
TeX source.

A remote image URL is never fetched by the page: it shows as a link with the host, because an agent's
output should not make the app load arbitrary addresses.

## Image viewer

Opened from attachments, images in replies, image-path chips, screenshot tool rows and the gallery.
Zoom with the buttons, `+`/`-`, `0` (fit), `1` (actual size) or ctrl/cmd + wheel; double-click toggles
2x; scroll to pan; arrow keys step through a set; Esc closes. **Copy** puts the PNG on the clipboard
(a browser needs a secure context for this), **Save** downloads it: the desktop app writes it to the
Downloads folder without overwriting an existing file, a browser downloads it, a phone opens the share
sheet.

## Gallery dock

Layout menu → Gallery (or the command palette): every screenshot the session's `browser` and `device`
tools captured, newest first, each with caption, tool and age. It is fed by the
[evidence ledger](evidence-ledger.md) and refreshes while open. A screenshot tool call also shows its
picture under its row, even when the tool group is folded.

## Daemon routes

All under the daemon token and confined to the session (`crates/forge-cli/src/serve_media.rs`):

- `GET /api/workspace/image?session=<id>&path=<p>[&w=<px>]`: an image inside the session's workspace
  or its evidence folder. Relative or absolute paths are accepted; `..`, `.git` and symlinks that leave
  the root are refused. The file's magic bytes decide whether it is an image (PNG, JPEG, GIF, WebP,
  HEIC, SVG), never its name, and anything over 20 MiB is refused.
- `GET /api/evidence?session=<id>`: the session's screenshots, oldest first (`file`, `ts`, `source`,
  `caption`, `url`, `size`).
- `GET /api/evidence/image?session=<id>&file=<name>[&w=<px>]`: one of them.

`w` (32 to 1024) returns a downsampled copy for thumbnails; at most two are decoded at a time so a
gallery opening does not stall the daemon, and an image over 64 megapixels is refused rather than
decoded. `GET /api/workspace/file` now also works for a session that is not live (it resolves the
folder from the stored session, like the git routes).
