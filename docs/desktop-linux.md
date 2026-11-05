# Forge Desktop on Linux: rendering and launch troubleshooting

Forge Desktop is a Tauri v2 app on WebKitGTK. Three WebKitGTK/driver defects and one packaging
interaction used to need a hand-written launcher script. The app now handles them itself, before
GTK or WebKit start (`mobile/src-tauri/src/linux_render.rs`). It never overwrites a variable you
set, logs what it did on stderr, and shows the same thing in **Settings -> Diagnostics** (and in
the About panel).

```
forge: linux webkit workarounds: session=wayland/hyprland gpu=nvidia[nvidia,display] webkitgtk=2.54 appimage=true set=[GDK_BACKEND=x11 (...); WEBKIT_DISABLE_DMABUF_RENDERER=1 (...)] kept=[] vblank_shim=on
```

## What is applied automatically

| Problem | Applied when | Effect |
| --- | --- | --- |
| Window never maps, `Gdk-Message: Error 71 (Protocol error)` on Hyprland/wlroots and on Wayland + NVIDIA | always (unless you set the variable) | `WEBKIT_DISABLE_DMABUF_RENDERER=1` |
| Black window on native Wayland with the NVIDIA proprietary driver and WebKitGTK 2.54 (the AppImage bundles 2.54; distros ship it too) | Wayland session, a proprietary-`nvidia` GPU that drives a connected output, WebKitGTK >= 2.54 and an X display (XWayland) reachable, and `GDK_BACKEND` unset | `GDK_BACKEND=x11` |
| Animation capped at 60 fps on NVIDIA: `drmWaitVBlank` answers `EOPNOTSUPP`, so WebKit falls back to a 16 ms timer | the driver refuses the call (NVIDIA); other drivers pass straight through | `drmWaitVBlank` shim built into the executable paces frames at the monitor's refresh (143 fps on 144 Hz) |
| AppImageLauncher intercepts the AppImage after an update | `$APPIMAGE` is set and the `appimage-type2` binfmt entry is enabled and belongs to AppImageLauncher | the type-2 magic of the updated AppImage is cleared (see below) |

The X11 fallback is deliberately narrow. XWayland costs fractional scaling and native global
shortcuts, so it is not applied to AMD, Intel, nouveau, hybrid laptops where NVIDIA drives no
output, or WebKitGTK older than 2.54. The 2.54 threshold comes from the one reproduction we have
(RTX 4090, driver 615, Hyprland); a distro that backports the fix will still get XWayland until the
threshold is raised.

## Opt-outs and overrides

| Variable | Effect |
| --- | --- |
| `FORGE_DESKTOP_NO_WEBKIT_WORKAROUNDS=1` | Apply nothing: no environment changes, no vblank shim, no AppImage magic edit. |
| `GDK_BACKEND=wayland` / `x11` | Your value is kept as is. |
| `FORGE_DESKTOP_GDK_BACKEND=wayland` / `x11` / `auto` | Same as `GDK_BACKEND` but only for Forge (`auto` is the default detection). Use `wayland` to refuse the NVIDIA X11 fallback. |
| `WEBKIT_DISABLE_DMABUF_RENDERER=0` | Keep WebKit's DMABUF renderer on (any value you set is kept; `0` means enabled). |
| `FORGE_DESKTOP_NO_VBLANK_SHIM=1` | Leave `drmWaitVBlank` alone (60 fps cap returns on NVIDIA). `FORGE_VBLANK_DEBUG=1` logs the first three calls. |
| `FORGE_DESKTOP_KEEP_APPIMAGE_MAGIC=1` | Never edit the AppImage file. |
| `FORGE_DESKTOP_AUTO_UPDATE=0` | Existing switch: no self-update. |

`WEBKIT_DISABLE_DMABUF_RENDERER=1` is still set on X11 sessions: it is the default earlier releases
shipped and the cost of dropping it is unmeasured on hosts we cannot test.

## AppImageLauncher

AppImageLauncher 2.2.x registers a kernel binfmt handler for every type-2 AppImage and runs it
through its own old runtime, which cannot read Forge's zstd-compressed image. Typical symptoms:
`This doesn't look like a squashfs image` or `fuse: memory allocation failed`. The interception
happens in the kernel before any Forge code runs, so the very first launch cannot be fixed from
inside the app, and `APPIMAGELAUNCHER_DISABLE` does not bypass it. Pick one:

- Clear the type-2 magic once (bytes 8-10 of the file) so the kernel runs the AppImage as a plain
  ELF with its own runtime:

  ```sh
  printf '\0\0\0' | dd of=~/Applications/Forge.AppImage bs=1 seek=8 conv=notrunc status=none
  ```

  After that, Forge keeps it working: when the updater installs a new AppImage the app clears the
  magic on the new file itself (only when the binfmt handler is AppImageLauncher's). Because the
  updater runs in the already-working process, updates do not break the setup.
- Or extract (`./Forge.AppImage --appimage-extract`) and run `squashfs-root/AppRun`. An extracted
  copy has no `$APPIMAGE`, so it does not self-update (by design; it would overwrite its own
  binary).
- Or remove AppImageLauncher / disable its binfmt entry (root).

An AppImage with the magic cleared is no longer integrated by AppImageLauncher; that is the point.
The packaging alternative (building the image with gzip/xz so the old runtime can read it) was
not adopted: it makes every download larger and cannot be verified without a release build.

## Reading the log

Run Forge from a terminal (or `journalctl --user` if launched by a unit) and look for the
`forge: linux webkit workarounds:` line. `set=[...]` lists what was applied and why, `kept=[...]`
the variables you provided, `notes=[...]` ignored or disabled settings.
