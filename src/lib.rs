mod app;
mod bridge;
mod global_state;
pub mod gui;
mod workspace;

use app::CodexGui;
use bridge::{start_app_server_bridge, start_dsh_app_server_bridge};
use codex_arg0::Arg0DispatchPaths;
use gpui::{
    App, AppContext as _, Bounds, Styled as _, TitlebarOptions, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, point, px, size, transparent_black,
};
use gpui_component::{Root, Theme};
use gpui_component_assets::Assets;
use gpui_platform::application;

pub use bridge::DshLaunchConfig;

#[cfg(not(target_family = "wasm"))]
pub fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("codex_gui=info,codex_app_server=info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

pub fn run_app(runtime: tokio::runtime::Handle, arg0_paths: Arg0DispatchPaths) {
    let bridge = start_app_server_bridge(runtime, arg0_paths);
    run_with_bridge(bridge, "codex-gui");
}

pub fn run_dsh_app(runtime: tokio::runtime::Handle, launch: DshLaunchConfig) {
    let bridge = start_dsh_app_server_bridge(runtime, launch);
    run_with_bridge(bridge, "dsh-gui");
}

fn run_with_bridge(
    (bridge, bridge_rx): (
        bridge::AppServerBridge,
        tokio::sync::mpsc::UnboundedReceiver<bridge::BridgeEvent>,
    ),
    window_title: &'static str,
) {
    application().with_assets(Assets).run(move |cx: &mut App| {
        gpui_component::init(cx);
        Theme::sync_system_appearance(None, cx);

        let bounds = Bounds::centered(None, size(px(1180.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: None,
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(9.), px(9.))),
                }),
                window_background: WindowBackgroundAppearance::Blurred,
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title(window_title);
                let view = cx.new(|cx| CodexGui::new(bridge, bridge_rx, window, cx));
                cx.new(|cx| Root::new(view, window, cx).bg(transparent_black()))
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
