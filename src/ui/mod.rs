pub(crate) mod locale;

use crate::{
    app::events::{AppEvent, Surface},
    connection::tls,
    storage::{
        preferences::{Settings, Theme},
        profiles::Profile,
    },
};
use anyhow::Result;
use tao::{event_loop::EventLoopProxy, window::Window};
use wry::{
    NewWindowResponse, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows,
};

pub const TOOLBAR_HEIGHT: f64 = 48.0;
pub const REVEAL_EDGE_HEIGHT: f64 = 3.0;
pub const LOCAL_ORIGIN: &str = "https://comet.localhost";
fn builder(context: &mut wry::WebContext) -> WebViewBuilder<'_> {
    WebViewBuilder::new_with_web_context(context)
        .with_incognito(true)
        .with_devtools(cfg!(debug_assertions))
        .with_browser_accelerator_keys(false)
        .with_default_context_menus(false)
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
}

pub fn control(
    window: &Window,
    context: &mut wry::WebContext,
    surface: Surface,
    proxy: EventLoopProxy<AppEvent>,
    settings: &Settings,
) -> Result<WebView> {
    let path = match surface {
        Surface::Toolbar => "toolbar",
        Surface::Manager => "manager",
    };
    let destination = format!("{LOCAL_ORIGIN}/{path}");
    let expected = destination.clone();
    let (bar, body) = layout(window, TOOLBAR_HEIGHT);
    let bounds = if surface == Surface::Toolbar {
        bar
    } else {
        body
    };
    let catalog = serde_json::to_string(locale::current())?;
    let settings_json = serde_json::to_string(settings)?;
    let init = format!(
        "Object.defineProperty(window,'cometI18n',{{value:{catalog}}});\n\
         Object.defineProperty(window,'cometSettings',{{value:{settings_json}}});"
    );
    let view = builder(context)
        .with_custom_protocol("comet".into(), |_, request| {
            let (content, kind): (&[u8], &str) = match request.uri().path() {
                "/toolbar" => (include_bytes!("toolbar.html"), "text/html; charset=utf-8"),
                "/manager" => (include_bytes!("manager.html"), "text/html; charset=utf-8"),
                "/manager.js" => (
                    include_bytes!("manager.js"),
                    "text/javascript; charset=utf-8",
                ),
                "/toolbar.js" => (
                    include_bytes!("toolbar.js"),
                    "text/javascript; charset=utf-8",
                ),
                "/controls.js" => (
                    include_bytes!("controls.js"),
                    "text/javascript; charset=utf-8",
                ),
                "/theme.css" => (include_bytes!("theme.css"), "text/css; charset=utf-8"),
                _ => (b"Not found", "text/plain"),
            };
            wry::http::Response::builder()
                .header("Content-Type", kind)
                .body(std::borrow::Cow::Borrowed(content))
                .unwrap()
        })
        .with_https_scheme(true)
        .with_url(format!("comet://localhost/{path}"))
        .with_bounds(bounds)
        .with_initialization_script(&init)
        .with_background_color(if settings.theme == Theme::Dark {
            (26, 26, 26, 255)
        } else {
            (245, 245, 247, 255)
        })
        .with_ipc_handler(move |request| {
            if request.uri().to_string() == expected && request.body().len() <= 65536 {
                let _ = proxy.send_event(AppEvent::Local {
                    surface,
                    message: request.body().clone(),
                });
            }
        })
        .with_navigation_handler(move |url| url == destination)
        .build_as_child(window)?;
    view.set_theme(settings.theme.webview())?;
    Ok(view)
}

pub fn remote(
    window: &Window,
    context: &mut wry::WebContext,
    profile: &Profile,
    token: &str,
    session: u64,
    proxy: EventLoopProxy<AppEvent>,
    theme: Theme,
) -> Result<WebView> {
    let origin = url::Url::parse(&profile.url)?.origin();
    let allowed_origin = serde_json::to_string(&origin.ascii_serialization())?;
    let token = serde_json::to_string(token)?;
    let bridge = include_str!("../keyboard/bridge.js");
    let script = format!(
        r#"
        Object.defineProperty(window, '__cometConnectionOrigin', {{value: {allowed_origin}}});
        if (window.top === window && location.origin === window.__cometConnectionOrigin) {{
            localStorage.setItem('gl-kvm-token-keys', JSON.stringify({{glkvm: {token}}}));
        }}
        {bridge}
    "#
    );
    let ipc_origin = origin.clone();
    let nav_origin = origin.clone();
    let nav_proxy = proxy.clone();
    // Install the certificate-error handler before the first device navigation.
    // The token initialization above runs only in the device's top-level origin.
    let view = builder(context)
        .with_url("about:blank")
        .with_bounds(layout(window, TOOLBAR_HEIGHT).1)
        .with_visible(false)
        .with_initialization_script(&script)
        .with_ipc_handler(move |request| {
            if tls::same_origin(&request.uri().to_string(), &ipc_origin)
                && request.body().len() < 4096
            {
                let _ = proxy.send_event(AppEvent::Remote {
                    session,
                    message: request.body().clone(),
                });
            }
        })
        .with_navigation_handler(move |url| {
            let _ = nav_proxy.send_event(AppEvent::Remote {
                session,
                message: "{\"kind\":\"ready\"}".into(),
            });
            url == "about:blank" || tls::same_origin(&url, &nav_origin)
        })
        .build_as_child(window)?;
    tls::install(
        &view,
        origin,
        profile.certificate_sha256.clone().unwrap_or_default(),
    )?;
    view.set_theme(theme.webview())?;
    view.load_url(&profile.url)?;
    Ok(view)
}
pub fn layout(window: &Window, visible_height: f64) -> (wry::Rect, wry::Rect) {
    let s = window.inner_size();
    bounds(s.width, s.height, window.scale_factor(), visible_height)
}
fn bounds(width: u32, height: u32, scale: f64, visible_height: f64) -> (wry::Rect, wry::Rect) {
    let visible = ((visible_height.clamp(REVEAL_EDGE_HEIGHT, TOOLBAR_HEIGHT) * scale).round()
        as u32)
        .min(height);
    // Keep the WebView's viewport at its full height even when hidden.
    // Only its position changes, so its controls never reflow into the edge.
    let bar_height = ((TOOLBAR_HEIGHT * scale).round() as u32).min(height);
    let rect = |top: i32, height| wry::Rect {
        position: wry::dpi::PhysicalPosition::new(0, top).into(),
        size: wry::dpi::PhysicalSize::new(width, height).into(),
    };
    (
        rect(visible as i32 - bar_height as i32, bar_height),
        rect(visible as i32, height.saturating_sub(visible)),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewports_never_overlap_at_any_dpi() {
        for scale in [1., 1.25, 1.5, 2.] {
            for (w, h) in [(1440, 900), (2560, 1440), (3840, 2160), (0, 0)] {
                for visible_height in [48., 23., 3.] {
                    let (a, b) = bounds(w, h, scale, visible_height);
                    let bar_top = a.position.to_physical::<i32>(scale).y;
                    let a = a.size.to_physical::<u32>(scale);
                    let top = b.position.to_physical::<i32>(scale);
                    let b = b.size.to_physical::<u32>(scale);
                    let visible = ((visible_height * scale).round() as u32).min(h);
                    assert_eq!(top.y, visible as i32);
                    assert_eq!(bar_top + a.height as i32, visible as i32);
                    assert_eq!(visible + b.height, h);
                    assert_eq!(b.width, w);
                    assert_eq!(a.height, ((TOOLBAR_HEIGHT * scale).round() as u32).min(h));
                }
            }
        }
    }
}
