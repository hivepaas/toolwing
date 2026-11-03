//! `toolwing`: the desktop app (docs/design/app.md).

pub mod hashes;
pub mod page;
pub mod workspace;

use std::borrow::Cow;

use gpui::App;
use gpui_component::Theme;

/// What the interface is written in, as in tracewing.
pub const FONT: &str = "Nunito";

/// What inputs, outputs and commands are shown in.
pub const MONO_FONT: &str = "JetBrains Mono";

const FONTS: [&[u8]; 5] = [
    include_bytes!("../assets/fonts/Nunito-Regular.ttf"),
    include_bytes!("../assets/fonts/Nunito-Medium.ttf"),
    include_bytes!("../assets/fonts/Nunito-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Nunito-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
];

/// Everything the window needs before it opens: the components and the fonts.
pub fn init(cx: &mut App) {
    gpui_component::init(cx);
    let fonts = FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("toolwing: cannot load the fonts: {e}");
    }
    let theme = Theme::global_mut(cx);
    theme.font_family = FONT.into();
    theme.mono_font_family = MONO_FONT.into();
}
