//! `toolwing`: the desktop app (docs/design/app.md).

use gpui::{AppContext as _, Bounds, WindowBounds, WindowDecorations, WindowOptions, px, size};
use gpui_component::{Root, TitleBar};

fn main() {
    gpui_platform::application()
        .with_assets(gpui_kit_assets::Assets)
        .run(|cx| {
            toolwing::init(cx);
            // GNOME's Wayland compositor draws no window frame, so Linux gets
            // client-side decorations, as in the other -wing apps.
            let client_side = cfg!(any(target_os = "linux", target_os = "freebsd"));
            let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
            let opened = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(720.), px(480.))),
                    window_decorations: client_side.then_some(WindowDecorations::Client),
                    app_id: Some("toolwing".into()),
                    focus: true,
                    ..TitleBar::window_options()
                },
                |window, cx| {
                    let view = cx.new(|cx| toolwing::workspace::Workspace::new(window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            );
            if let Err(e) = opened {
                eprintln!("toolwing: cannot open a window: {e:#}");
                cx.quit();
                return;
            }
            cx.on_window_closed(|cx, _| cx.quit()).detach();
            cx.activate(true);
        });
}
