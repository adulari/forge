// Hearth: settings-family routes bring their own 240px nav rail (SettingsShell), so the
// persistent Fleet rail — and with it every workbench pane and dock — steps aside there: one
// rail on screen at a time. Connect is a full-bleed pairing screen on every surface.
export const RAILLESS_ROUTES = /^\/(settings|appearance|keybindings|diagnostics|perf-fixture|legal|configuration|skills|hooks|providers|models|plans|mcp|usage|session-tree|workflows|gallery|connect|anywhere|shares|onboarding)(\/|$)/;
