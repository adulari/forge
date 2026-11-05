//! Machined dark tokens from mobile/src/theme/tokens.ts, mapped onto the component theme.
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::{rgb, rgba, App, Hsla};

pub const BG0: u32 = 0x09090B;
pub const BG1: u32 = 0x0D0D11;
pub const BG3: u32 = 0x101015;
pub const INK: u32 = 0xF4F4F6;
pub const INK2: u32 = 0x9A9AA6;
pub const INK3: u32 = 0x5F5F6B;
pub const INK4: u32 = 0x45454F;
pub const ACCENT: u32 = 0xFF8A3D;
pub const SUCCESS: u32 = 0x5FB97D;
pub const DANGER: u32 = 0xE5605C;

pub fn border() -> Hsla {
    rgba(0xF4F4F617).into()
}

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub fn apply(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    theme.background = c(BG0);
    theme.foreground = c(INK);
    theme.border = border();
    theme.muted = c(BG3);
    theme.muted_foreground = c(INK2);
    theme.primary = c(ACCENT);
    theme.input = border();
    theme.selection = rgba(0xFF8A3D24).into();
    theme.list_active = rgba(0xF4F4F613).into();
    theme.list_hover = rgba(0xF4F4F60B).into();
    theme.popover = c(BG1);
    theme.scrollbar_thumb = rgba(0xF4F4F62E).into();
    theme.font_family = "Geist".into();
    theme.mono_font_family = "Geist Mono".into();
    theme.font_size = gpui_kit::px(13.0);
    theme.mono_font_size = gpui_kit::px(12.0);
}

#[allow(dead_code)]
pub fn active(cx: &App) -> &Theme {
    cx.theme()
}
