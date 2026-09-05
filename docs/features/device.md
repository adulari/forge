# Android device control

Forge can drive a real phone or emulator over adb: boot it, install an app, tap through a flow,
read what is on screen, and read the log when it breaks.

This is the mobile counterpart to [browser control](browser.md). The reasoning is the same — an
agent that can only reason *about* an app guesses, while an agent that can drive it observes — and
it completes the mobile testing loop opened by the proxy: `proxy` shows what the app sent, `device`
drives the app that sent it, and `device_logs` says why it failed.

## Tools

Two tools, not a dozen: every tool schema sits in the system prompt of every turn, so the surface
is deliberately small. `device` controls, `device_logs` inspects.

### `device`

`action` is one of `devices`, `info`, `avds`, `emulator_start`, `emulator_stop`, `install`,
`uninstall`, `launch`, `stop`, `clear`, `packages`, `current`, `tap`, `long_press`, `swipe`,
`text`, `key`, `screenshot`, `ui`, `find`, `wait_for`, `shell`, `push`, `pull`, `proxy_set`,
`proxy_clear`, `reverse`, `ca_install`.

| argument | meaning |
| --- | --- |
| `serial` | which device. Optional when exactly one is connected, required when several are |
| `avd`, `headless`, `wipe_data`, `cold_boot` | for `emulator_start` |
| `memory_mb`, `cores`, `gpu`, `writable_system` | emulator resources and mode |
| `package`, `activity`, `apk`, `grant`, `reinstall` | app lifecycle |
| `x`, `y`, `x2`, `y2`, `duration_ms` | coordinates, for `tap` / `long_press` / `swipe` |
| `text`, `resource_id`, `content_desc`, `class`, `exact`, `clickable_only` | the selector |
| `key` | `back`, `home`, `enter`, `app_switch`, … or a raw `KEYCODE_*` |
| `command` | for `shell` |
| `local`, `remote` | for `push` / `pull` |
| `endpoint`, `port`, `cert`, `user_store` | proxy and certificate wiring |
| `all`, `max_chars`, `timeout_secs` | output size and patience |

### `device_logs`

`action` is `read` or `clear`. Filter with `package`, `tag`, `level`
(`verbose`…`fatal`), `contains` and `limit`; an unfiltered buffer is tens of thousands of lines.

The useful pattern is `clear` before an action and `read` after it, so the output is only what that
action produced.

## Selectors, not pixels

`ui` returns every on-screen element with its class, text, content description, resource id, flags
and exact tap point:

```
com.example.app — 14 elements
  EditText "Email" id=email [clickable] @540,560
  Button "Log in" desc="Log in to your account" id=login_button [clickable] @540,760
```

`tap`, `find` and `wait_for` take the same selector fields, so a flow reads as
`tap {text: "Log in"}` rather than `tap {x: 540, y: 760}`. That survives a different screen size, a
relocated button, and a re-render; a coordinate does not. `resource_id` matches the bare id
(`login_button`) as well as the package-qualified form, because that is how the id appears in the
app's own source.

Coordinates still win when both are given — some things genuinely have no accessible node.

Layout-only nodes are hidden by default: a real screen is a few hundred nodes of which a dozen
matter, and the rest is `FrameLayout` nesting. Pass `all: true` when you need the full tree.

## Waiting

`wait_for` polls the hierarchy until an element appears, which is what makes a flow reliable across
a network call or an animation. A dump that fails mid-animation is retried rather than reported;
only running out of time is an error.

## The device is persistent

An app installed in one session is still installed in the next. Nothing in the default boot erases
state: `wipe_data` is off, so the userdata image survives, and the quick-boot snapshot is both
loaded and saved, so a session starts where the last one ended.

That last part has a trap in it. The emulator writes its state on a *graceful* shutdown, which is
what `emulator_stop` performs. Killing the process instead — `kill -9`, closing a terminal that
owns it — discards everything since the last save, and for an app installed minutes earlier that
means the app. Always stop through `emulator_stop`.

`cold_boot: true` ignores the snapshot but keeps installed apps, since those live in the userdata
image rather than the snapshot. `wipe_data: true` is the only option that genuinely erases the
device, and it is never a default.

## The emulator is deliberately small

`emulator_start` boots with 2 GB and 2 cores, no audio, no boot animation, no metrics upload, and
software GL when headless. An emulator on its own defaults takes the host's core count and competes
with the build it is supposed to be testing.

Raise `memory_mb` / `cores` for a heavier app, or set them to use the AVD's own configuration.
Quick-boot snapshots are used by default so a repeat boot is seconds rather than a minute; pass
`cold_boot: true` when a stale snapshot would confuse the test, or `wipe_data: true` for a genuine
first-run state.

`emulator_stop` waits for the device to actually disappear from the device list rather than
returning as soon as the kill is sent — the process takes seconds to flush its disk image, and a
boot started in that window collides with the shutdown still in progress.

## Seeing the app's traffic

`proxy_set` points the device's HTTP(S) traffic at a proxy; with `proxy` running, that is Forge's
own. Two things commonly go wrong, and both have a specific answer:

- **The proxy is on localhost.** `reverse` opens an adb tunnel from the device back to this
  machine, which is the only thing that works when the proxy is not exposed on the LAN.
- **HTTPS fails.** The app does not trust the interception CA. `ca_install` writes it into the
  **system** trust store, which needs root — in practice an emulator booted with
  `writable_system: true` on a Google APIs (not Play) image. `user_store: true` stages it into the
  user store instead, which needs no root but, since Android 7, is only trusted by apps that opt
  in.

`emulator_start` also accepts `endpoint`, which sets `-http-proxy` at the emulator level. That
catches apps that ignore the system proxy setting.

## Permissions

Both tools declare `SideEffect::Shell`, not `Network`. That is the honest class: `shell` runs
arbitrary commands on the device, `install` puts software on it, and `clear` destroys an app's
data. They are gated exactly like a shell command.

## Limits

- `text` types through `input text`, which is ASCII-only. Non-ASCII is rejected up front rather
  than silently typing something else. Spaces and shell metacharacters are escaped for you.
- `ui` needs `uiautomator`, which refuses to dump a secure window (a password field flagged
  `FLAG_SECURE`, a payment sheet). Those screens can be screenshotted but not read.
- A dump is a snapshot. An element found and then tapped in two calls can move in between; prefer
  `tap` with a selector, which resolves and taps in one call.
- `ca_install` needs `openssl` on the host to compute the trust-store filename.

## Testing

Unit tests cover the parsing and escaping — the adb output shapes, the hierarchy XML, the
`input text` escapes, the boot flags. What they cannot cover is adb's real behaviour, so
`crates/forge-device/tests/live_device.rs` drives an actual device:

```sh
FORGE_DEVICE_LIVE=1 cargo test -p forge-agent-device --test live_device -- --test-threads=1
```

It boots a light emulator on demand if none is connected. `FORGE_DEVICE_LIVE_BOOT=1` additionally
runs the boot/shutdown lifecycle test. Both are skipped by default, because CI has no phone.
