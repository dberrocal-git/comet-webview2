//! Window-thread coordinator for connection state, child views and keyboard input.
//!
//! `App` is mutated only by Tao's event loop. WebView callbacks and login workers
//! enqueue `AppEvent`s instead of sharing mutable state. `commands` handles local
//! UI actions, `login` owns pending authentication, and `layout` positions views.
//! See `../DEVELOPMENT.md` for lifecycle diagrams and the Rust/JS wire contracts.

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

/// Limited telemetry/capture protocol accepted from the current device viewer.
/// These messages cannot invoke profile, settings or certificate-trust commands.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RemoteMessage {
    Capture { enabled: bool },
    Ready,
    Stats(ViewerStats),
}

/// Firmware bridge observations, not proof of authentication or HID delivery.
/// A control socket can be connected with no HDMI signal (`width == 0`).
#[derive(Default, Deserialize)]
struct ViewerStats {
    connected: bool,
    width: u32,
    hid_packets: u64,
    #[serde(default)]
    browser_lock_requests: u64,
    #[serde(default)]
    dom_key_events: u64,
}
// Cover the 150 ms page exit in ui/manager.html, with a small margin before hiding.
const MANAGER_EXIT_ANIMATION: Duration = Duration::from_millis(160);

// Keep the window and its identity together. A new login may be pending
// while callbacks from the currently displayed viewer are still valid.
struct Viewer {
    session: u64,
    profile_id: String,
    url: String,
    view: wry::WebView,
}

/// All native application state. Selected, pending and displayed profiles may
/// differ: editing or connecting must not silently retarget a live viewer.
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
    // Editor selection, distinct from Viewer.profile_id and PendingLogin.profile.
    selected: Option<String>,
    toolbar_revealed: bool,
    toolbar_height: f64,
    toolbar_animation: Option<ToolbarAnimation>,
    // Logical visibility stays true until the outgoing manager animation ends.
    manager_close_at: Option<Instant>,
    // Increment only to request an editor refill; routine publish must preserve edits.
    form_version: u64,
    manager_visible: bool,
    // True for both exclusive DirectInput and the DOM typing fallback. The user's
    // persistent permission is preferences.settings.keyboard_capture instead.
    capture_requested: bool,
    stats: ViewerStats,
    next_attempt: u64,
    pending: Option<PendingLogin>,
    notice: Option<String>,
    // Grace period for remote key releases during shutdown; there is no delivery ack.
    close_at: Option<Instant>,
    diagnostics_path: Option<PathBuf>,
    last_diagnostics: Instant,
    forwarded_keys: u64,
}
/// Initialize Windows/WebView2 resources, then run until the window closes.
/// Startup errors reach main's native dialog; per-command errors use the manager.
pub fn run() -> Result<()> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context(tr("error.local_app_data"))?;
    let root = local_app_data.join("Comet-WebView2");
    // Settings must load before locale selection. Errors this early use bundled
    // English through locale::current's pre-initialization fallback.
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
        // Poll buffered native input frequently while acquired; animations need
        // frames, but an idle window can sleep longer. This is not a fixed FPS loop.
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
                // Minimize the lifetime of the queued JSON copy containing a
                // submitted password; this does not erase copies in WebView2/JS.
                let message = Zeroizing::new(message);
                if let Err(error) = app.command(surface, &message) {
                    app.notice = Some(format!("{error:#}"));
                    app.show_manager();
                    app.publish();
                }
            }
            Event::UserEvent(AppEvent::Remote { session, message }) => {
                // Callbacks can already be queued when a viewer is replaced.
                // Compare with the displayed viewer, not the latest login attempt.
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
    /// Best-effort, opt-in health snapshot; counters do not log keys or secrets.
    /// Diagnostic write errors must not interrupt a working KVM session.
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
    /// Release input before exposing local form controls; keep the viewer alive.
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
    /// Start the manager exit, or switch immediately if OS animations are disabled.
    fn show_remote(&mut self) {
        if self.manager_visible && animations_enabled() {
            let _ = self.manager.evaluate_script("window.cometManager?.leave()");
            self.manager_close_at = Some(Instant::now() + MANAGER_EXIT_ANIMATION);
        } else {
            self.finish_show_remote();
        }
    }
    /// Complete the visibility switch and focus the viewer, without arming capture.
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
    /// Ignore malformed/unknown firmware payloads; never treat them as local commands.
    fn remote_message(&mut self, message: &str) {
        match serde_json::from_str::<RemoteMessage>(message) {
            Ok(RemoteMessage::Capture { enabled }) => {
                self.click_capture(enabled);
            }
            Ok(RemoteMessage::Ready) => {
                // Navigation or a newly mounted document invalidates old bridge
                // observations and requires a fresh click to capture.
                self.stats = ViewerStats::default();
                self.release();
                self.update_toolbar();
            }
            Ok(RemoteMessage::Stats(stats)) => {
                self.stats = stats;
                if !self.stats.connected {
                    self.release();
                }
                self.update_toolbar();
            }
            Err(_) => {}
        }
    }
    /// Arbitrate a capture request against preference, connection and window state.
    /// `enabled` alone is intent, not permission from the remote document.
    fn click_capture(&mut self, enabled: bool) {
        if !enabled
            || !self.preferences.settings.keyboard_capture
            || self.manager_visible
            || !self.stats.connected
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
        // Reset first so a previous DOM fallback cannot leave remote modifiers
        // held when the user retries exclusive acquisition.
        let _ = viewer
            .view
            .evaluate_script(&format!(
                "window.__cometNative?.setCaptured(false);window.__cometNative?.setCaptured(true,{basic})"
            ));
        self.update_toolbar();
    }

    /// Drain ordered physical transitions and send a single JS batch per poll.
    /// Any read error ends capture: a lost key-up could otherwise stick remotely.
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

    /// Release both owners: the native device and the bridge's remote held keys.
    /// Keep repeated calls harmless and leave the persistent preference unchanged.
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
    /// Publish connection/capture status separately from the profile/editor state.
    fn update_toolbar(&self) {
        let state = serde_json::json!({
            "online": self.stats.connected,
            "captured": self.capture.enabled(),
            "basic": self.capture_requested && !self.capture.enabled(),
            "capture_enabled": self.preferences.settings.keyboard_capture,
        });
        let _ = self
            .toolbar
            .evaluate_script(&format!("window.cometToolbar?.update({state})"));
    }
    /// Send a complete local UI snapshot using public profiles only.
    /// JS refills the form only when form_version changes, preserving ongoing edits.
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
    /// Match the live session to a saved profile only while its URL still agrees.
    /// Editing/deleting a profile does not disconnect or relabel its existing viewer.
    fn viewer_id(&self) -> Option<&str> {
        let viewer = self.remote.as_ref()?;
        self.store
            .find(&viewer.profile_id)
            .filter(|profile| profile.url == viewer.url)
            .map(|profile| profile.id.as_str())
    }
}
