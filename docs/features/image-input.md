# Image input — clipboard, dropped files, `@path`, `/image`

> Status: **shipped.** Everything here produces the same thing: an image block in the prompt
> (`[image (shot.png PNG 1280x720 84KB)]`) that is sent to the model as vision input.

## Ways to attach an image

| Gesture | What happens |
| --- | --- |
| `Ctrl+V` | Read an image off the clipboard; if there is none, start `/voice` exactly as before. |
| Empty paste (some terminals send this for an image) | Same clipboard read; when nothing is found a dim note says why. |
| Paste or drag a file path | Images attach as image blocks, text/code files become `@path` mentions. Quoted, backslash-escaped, `file://` and `~` forms all work, several paths at once too. |
| Type a path and press Enter | A line that is only file paths attaches instead of being sent; add your words and press Enter again. |
| `@shot.png` in a prompt | Attached as an image (previously reported as `(binary)`). |
| `/image <path>` | Same loader; accepts quotes, `~`, `file://`. |

PDF paths are recognised but not yet attachable (a note says so); a document attachment type
is future work.

A path that is under `/home/<user>` or `/Users/<user>` and does not exist on this machine (a
laptop path pasted into a remote session) is reported as "not on this machine" instead of being
silently inserted.

## Clipboard backends

Tried in order, each with a 2 second timeout, first image wins:

1. macOS: `pngpaste -`, then `osascript` (`the clipboard as «class PNGf»`).
2. Windows: PowerShell `System.Windows.Forms.Clipboard`.
3. Wayland (`WAYLAND_DISPLAY` set): `wl-paste --list-types`, then `wl-paste --type image/png|jpeg|webp|gif|bmp`.
4. X11 (`DISPLAY` set): `xclip -selection clipboard -t TARGETS -o`, then the same target read.
5. In-process `arboard` (last resort, also under a 2 s deadline).

Inside tmux, the process `DISPLAY` is often stale after an ssh reconnect, so Forge reads the
current value with `tmux show-environment DISPLAY` first (read-only; the tmux server is not
modified).

If nothing is found the prompt shows one dim line saying why: no `DISPLAY`/`WAYLAND_DISPLAY`,
`xclip` not installed, a backend timing out, or simply no image on the clipboard.

### Remote server recipe (tmux over `ssh -X`)

1. On the laptop: `ssh -X host` (or `ForwardX11 yes` in `~/.ssh/config`). A Wayland laptop needs
   XWayland running, which Hyprland starts by default.
2. On the server install `xclip` (any location on `PATH`, `~/.local/bin` works) and make sure
   `sshd` has `X11Forwarding yes`.
3. Copy a screenshot on the laptop (`wl-copy < shot.png`, or your screenshot tool's "copy"),
   focus Forge, press `Ctrl+V`.

Check the path by hand: `xclip -selection clipboard -t TARGETS -o` on the server should list
`image/png`. Without X forwarding, `scp` the file over and drop its path, or use `/image <path>`.

## Formats and size

The format is read from the file's magic bytes, never the extension: png, jpeg, gif, webp,
heic/heif. HEIC/HEIF is converted to JPEG through the first working of `heif-convert`, `magick`,
`sips` (macOS), `convert`; with none installed the error says what to install.

Images whose longest side is over 2000 px, or whose encoded size is over 3 MB, are decoded
(EXIF rotation applied), shrunk to at most 2000 px and re-encoded: PNG for screenshot-like
images (few colours, or any transparency), JPEG quality 85 for photographs. Smaller images are
sent byte for byte. The raw file cap is 20 MB.

## Prompts typed while a turn is running

A prompt with an image is queued together with its images and runs, images attached, when the
current turn finishes. Unlike text-only prompts it is not steered into the running turn (a
steered line cannot carry pixels). An image with no words is kept in the input box.
