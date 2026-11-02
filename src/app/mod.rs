mod commands;
pub(crate) mod events;
mod layout;
mod login;

use layout::{ToolbarAnimation, animations_enabled};
use login::PendingLogin;

use self::events::{AppEvent, Surface};
use crate::{
    keyboard,
    storage::{
        preferences::Preferences,
        profiles::{Profile, Store},
    },
    ui::{
        self,
        locale::{self, tr},
    },
};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::windows::{IconExtWindows, WindowBuilderExtWindows, WindowExtWindows},
    window::{Icon, Window, WindowBuilder},
};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RemoteMessage {
    Capture { enabled: bool },
    Ready,
    Stats(ViewerStats),
}

#[derive(Default, Deserialize)]
struct ViewerStats {
    connected: bool,
    width: u32,
    height: u32,
    hid_packets: u64,
    #[serde(default)]
    browser_lock_requests: u64,
    #[serde(default)]
    dom_key_events: u64,
}
const MANAGER_EXIT_ANIMATION: Duration = Duration::from_millis(160);

// Keep the window and its identity together. A new login may be pending
// while callbacks from the currently displayed viewer are still valid.
struct Viewer {
    session: u64,
    profile_id: String,
    url: String,
    view: wry::WebView,
}

struct App {
    window: Window,
    context: wry::WebContext,
    toolbar: wry::WebView,
    manager: wry::WebView,
    remote: Option<Viewer>,
    capture: keyboard::Capture,
    proxy: EventLoopProxy<AppEvent>,
    store: Store,
    preferences: Preferences,
    selected: Option<String>,
    toolbar_revealed: bool,
    toolbar_height: f64,
    toolbar_animation: Option<ToolbarAnimation>,
    manager_close_at: Option<Instant>,
    form_version: u64,
    manager_visible: bool,
    capture_requested: bool,
    stats: ViewerStats,
    next_attempt: u64,
    pending: Option<PendingLogin>,
    notice: Option<String>,
    close_at: Option<Instant>,
    diagnostics_path: Option<PathBuf>,
    last_diagnostics: Instant,
    forwarded_keys: u64,
}
pub fn run() -> Result<()> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context(tr("error.local_app_data"))?;
    let root = local_app_data.join("Comet-WebView2");
    let preferences = Preferences::open(root.join("settings.json"))?;
    locale::init(&preferences.settings.language);
    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_window_icon(Icon::from_resource(1, Some(small_icon_size())).ok())
        .with_taskbar_icon(Icon::from_resource(1, None).ok())
        .with_theme(Some(preferences.settings.theme.native()))
        .with_title(tr("app.name"))
        .with_inner_size(LogicalSize::new(1440., 900.))
        .with_min_inner_size(LogicalSize::new(850., 560.))
        .build(&event_loop)?;
    let store = Store::open(root.join("connections.json"))?;
    let selected = store
        .last()
        .map(String::from)
        .or_else(|| store.all().first().map(|p| p.id.clone()));
    let mut context = wry::WebContext::new(Some(root.join("WebView2")));
    let proxy = event_loop.create_proxy();
    let capture = keyboard::Capture::new(window.hwnd());
    let mut control_view = |surface| {
        ui::control(
            &window,
            &mut context,
            surface,
            proxy.clone(),
            &preferences.settings,
        )
    };
    let toolbar = control_view(Surface::Toolbar)?;
    let manager = control_view(Surface::Manager)?;
    let mut app = App {
        window,
        context,
        toolbar,
        manager,
        remote: None,
        capture,
        proxy,
        store,
        preferences,
        selected,
        toolbar_revealed: false,
        toolbar_height: ui::TOOLBAR_HEIGHT,
        toolbar_animation: None,
        manager_close_at: None,
        form_version: 1,
        manager_visible: true,
        capture_requested: false,
        stats: ViewerStats::default(),
        next_attempt: 0,
        pending: None,
        notice: None,
        close_at: None,
        diagnostics_path: std::env::var_os("COMET_KEYBOARD_DIAGNOSTICS")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::args()
                    .any(|arg| arg == "--keyboard-diagnostics")
                    .then(|| root.join("keyboard-health.json"))
            }),
        last_diagnostics: Instant::now(),
        forwarded_keys: 0,
    };
    event_loop.run(move |event, _, flow| {
        let now = Instant::now();
        let next_frame = now
            + Duration::from_millis(if app.capture.enabled() {
                8
            } else if app.toolbar_animation.is_some() {
                16
            } else {
                100
            });
        *flow = ControlFlow::WaitUntil(
            app.manager_close_at
                .map_or(next_frame, |deadline| deadline.min(next_frame)),
        );
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                app.release();
                app.pending = None;
                app.close_at = Some(Instant::now() + Duration::from_millis(300));
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. },
                ..
            } => app.resize(),
            Event::WindowEvent {
                event: WindowEvent::Focused(false),
                ..
            } => {
                // Moving focus into a WebView child also blurs the host window.
                // Only leaving our top-level window should release held keys.
                if !app.capture.is_foreground() {
                    app.release();
                }
            }
            Event::UserEvent(AppEvent::Local { surface, message }) => {
                let message = Zeroizing::new(message);
                if let Err(error) = app.command(surface, &message) {
                    app.notice = Some(format!("{error:#}"));
                    app.show_manager();
                    app.publish();
                }
            }
            Event::UserEvent(AppEvent::Remote { session, message }) => {
                if app
                    .remote
                    .as_ref()
                    .is_some_and(|viewer| viewer.session == session)
                {
                    app.remote_message(&message);
                }
            }
            Event::UserEvent(AppEvent::Connection { attempt, outcome }) => {
                app.connection_result(attempt, outcome)
            }
            Event::MainEventsCleared => {
                if app.close_at.is_some_and(|at| Instant::now() >= at) {
                    *flow = ControlFlow::Exit;
                    return;
                }
                app.pump_keyboard();
                app.animate_toolbar();
                if app
                    .manager_close_at
                    .is_some_and(|deadline| Instant::now() >= deadline)
                {
                    app.finish_show_remote();
                }
                if app.last_diagnostics.elapsed() >= Duration::from_secs(1) {
                    app.write_diagnostics();
                    app.last_diagnostics = Instant::now();
                }
            }
            _ => {}
        }
    });
}

fn small_icon_size() -> tao::dpi::PhysicalSize<u32> {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON, SM_CYSMICON};
    unsafe {
        tao::dpi::PhysicalSize::new(
            GetSystemMetrics(SM_CXSMICON) as u32,
            GetSystemMetrics(SM_CYSMICON) as u32,
        )
    }
}

impl App {
    fn write_diagnostics(&self) {
        if let Some(path) = &self.diagnostics_path {
            let value = serde_json::json!({
                "process_id": std::process::id(),
                "capture": {
                    "exclusive": self.capture.enabled(),
                    "alt": self.capture.alt,
                    "acquisitions": self.capture.acquisitions,
                    "last_error": self.capture.last_error,
                },
                "capture_requested": self.capture_requested,
                "capture_enabled": self.preferences.settings.keyboard_capture,
                "manager_visible": self.manager_visible,
                "video_ready": self.stats.width > 0,
                "native_forwarded_events": self.forwarded_keys,
                "firmware_hid_packets": self.stats.hid_packets,
                "browser_lock_requests": self.stats.browser_lock_requests,
                "dom_key_events": self.stats.dom_key_events,
            });
            let _ = crate::storage::write_json(path, &value);
        }
    }
    fn show_manager(&mut self) {
        let enter = !self.manager_visible || self.manager_close_at.is_some();
        self.manager_close_at = None;
        self.release();
        self.toolbar_revealed = false;
        self.manager_visible = true;
        if let Some(viewer) = &self.remote {
            let _ = viewer.view.set_visible(false);
        }
        let _ = self.manager.set_visible(true);
        if enter {
            let _ = self.manager.evaluate_script("window.cometManager?.enter()");
        }
        let _ = self.manager.focus();
    }
    fn show_remote(&mut self) {
        if self.manager_visible && animations_enabled() {
            let _ = self.manager.evaluate_script("window.cometManager?.leave()");
            self.manager_close_at = Some(Instant::now() + MANAGER_EXIT_ANIMATION);
        } else {
            self.finish_show_remote();
        }
    }
    fn finish_show_remote(&mut self) {
        self.manager_close_at = None;
        if let Some(viewer) = &self.remote {
            self.manager_visible = false;
            let _ = self
                .manager
                .evaluate_script("window.cometManager?.prepare()");
            let _ = self.manager.set_visible(false);
            let _ = viewer.view.set_visible(true);
            let _ = viewer.view.focus();
            self.publish();
        }
    }
    fn remote_message(&mut self, message: &str) {
        match serde_json::from_str::<RemoteMessage>(message) {
            Ok(RemoteMessage::Capture { enabled }) => {
                self.click_capture(enabled);
            }
            Ok(RemoteMessage::Ready) => {
                self.stats = ViewerStats::default();
                self.release();
                self.update_toolbar();
            }
            Ok(RemoteMessage::Stats(stats)) => {
                self.stats = stats;
                if self.stats.width == 0 || self.stats.height == 0 {
                    self.release();
                }
                self.update_toolbar();
            }
            Err(_) => {}
        }
    }
    fn click_capture(&mut self, enabled: bool) {
        if !enabled
            || !self.preferences.settings.keyboard_capture
            || self.manager_visible
            || self.stats.width == 0
            || self.close_at.is_some()
            || !self.capture.is_foreground()
        {
            self.release();
            return;
        }
        if self.capture.enabled() {
            return;
        }
        let Some(viewer) = &self.remote else {
            return;
        };
        let _ = viewer.view.focus();
        self.capture.acquire();
        self.capture_requested = true;
        // Basic mode keeps typing available if exclusive acquisition failed.
        let basic = !self.capture.enabled();
        let _ = viewer
            .view
            .evaluate_script(&format!("window.__cometNative?.setCaptured(true,{basic})"));
        self.update_toolbar();
    }

    fn pump_keyboard(&mut self) {
        if !self.capture_requested {
            return;
        }
        if !self.capture.is_foreground() {
            self.release();
            return;
        }
        match self.capture.read() {
            Ok(keys) if !keys.is_empty() => {
                if let Some(viewer) = &self.remote {
                    self.forwarded_keys += keys.len() as u64;
                    let batch = serde_json::to_string(&keys).unwrap();
                    let _ = viewer
                        .view
                        .evaluate_script(&format!("window.__cometNative?.batch({batch})"));
                }
            }
            Ok(_) => {}
            Err(error) => {
                self.capture.last_error = Some(error.to_string());
                self.release();
            }
        }
    }

    fn release(&mut self) {
        let was_active = self.capture_requested;
        self.capture_requested = false;
        self.capture.release();
        if was_active {
            if let Some(viewer) = &self.remote {
                let _ = viewer
                    .view
                    .evaluate_script("window.__cometNative?.setCaptured(false)");
            }
            self.update_toolbar();
        }
    }
    fn update_toolbar(&self) {
        let state = serde_json::json!({
            "online": self.stats.connected,
            "captured": self.capture.enabled(),
            "basic": self.capture_requested && !self.capture.enabled(),
            "capture_enabled": self.preferences.settings.keyboard_capture,
            "width": if self.manager_visible { 0 } else { self.stats.width },
        });
        let _ = self
            .toolbar
            .evaluate_script(&format!("window.cometToolbar?.update({state})"));
    }
    fn publish(&mut self) {
        self.resize();
        let state = serde_json::json!({
            "profiles": self.store.all().iter().map(Profile::public).collect::<Vec<_>>(),
            "selected_id": self.selected,
            "viewer_id": self.viewer_id(),
            "form_version": self.form_version,
            "busy": self.pending.is_some(),
            "notice": self.notice,
            "trust": self.pending.as_ref().and_then(PendingLogin::trust_prompt),
            "can_return": self.remote.is_some(),
            "manager_visible": self.manager_visible,
        });
        let _ = self
            .manager
            .evaluate_script(&format!("window.cometManager?.update({state})"));
        let _ = self
            .toolbar
            .evaluate_script(&format!("window.cometToolbar?.profiles({state})"));
        self.update_toolbar();
    }
    fn viewer_id(&self) -> Option<&str> {
        let viewer = self.remote.as_ref()?;
        self.store
            .find(&viewer.profile_id)
            .filter(|profile| profile.url == viewer.url)
            .map(|profile| profile.id.as_str())
    }
}
