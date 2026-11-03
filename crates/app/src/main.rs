//! `toolwing`: the desktop app (docs/design/app.md).

use gpui::{AppContext as _, Bounds, WindowBounds, WindowDecorations, WindowOptions, px, size};
use gpui_component::{Root, TitleBar};

fn main() {
    // The app's own binary is also its jq worker (jq_worker.rs).
    if std::env::args().nth(1).as_deref() == Some(toolwing::jq_worker::WORKER_ARG) {
        if let Err(e) = toolwing::jq_worker::serve() {
            eprintln!("toolwing: jq worker: {e}");
            std::process::exit(1);
        }
        return;
    }
    gpui_platform::application()
        .with_assets(gpui_kit_assets::Assets)
        .run(|cx| {
            toolwing::init(cx);
            // Started now, so the jq page's first run does not wait for it.
            toolwing::jq_page::start_worker(toolwing::jq_worker::Worker::current_exe(), cx);
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
