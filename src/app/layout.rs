//! Native child-WebView positioning and toolbar animation.
//!
//! Rust owns the actual bounds and timing; toolbar JS requests reveal/hide and
//! receives layout flags for presentation. Heights here are logical pixels;
//! `ui::layout` converts them to physical pixels using the window's current DPI.

use super::App;
use crate::ui;
use std::time::{Duration, Instant};

const TOOLBAR_ANIMATION: Duration = Duration::from_millis(160);

/// Interpolates visible toolbar height, including reversals partway through.
pub(super) struct ToolbarAnimation {
    from: f64,
    to: f64,
    started: Instant,
}
impl ToolbarAnimation {
    /// Cubic ease-out, clamped to finish exactly at the requested height.
    fn sample(&self, now: Instant) -> (f64, bool) {
        let progress = (now.saturating_duration_since(self.started).as_secs_f64()
            / TOOLBAR_ANIMATION.as_secs_f64())
        .clamp(0., 1.);
        let eased = 1. - (1. - progress).powi(3);
        (self.from + (self.to - self.from) * eased, progress >= 1.)
    }
}

impl App {
    pub(super) fn auto_hide_toolbar(&self) -> bool {
        !self.manager_visible && !self.preferences.settings.toolbar_pinned
    }
    /// Reconcile the target height and publish semantic layout flags to JS.
    pub(super) fn resize(&mut self) {
        let auto_hide = self.auto_hide_toolbar();
        let target = if auto_hide && !self.toolbar_revealed {
            ui::REVEAL_EDGE_HEIGHT
        } else {
            ui::TOOLBAR_HEIGHT
        };
        let now = Instant::now();
        // Sample before changing direction to avoid jumping to the old endpoint.
        self.advance_toolbar(now);
        let current_target = self
            .toolbar_animation
            .as_ref()
            .map(|animation| animation.to)
            .unwrap_or(self.toolbar_height);
        if (target - current_target).abs() > f64::EPSILON {
            if !self.manager_visible && animations_enabled() {
                self.toolbar_animation = Some(ToolbarAnimation {
                    from: self.toolbar_height,
                    to: target,
                    started: now,
                });
            } else {
                self.toolbar_height = target;
                self.toolbar_animation = None;
            }
        }
        self.apply_toolbar_bounds();
        // CSS enters its collapsed state only after the native slide finishes.
        let collapsed = auto_hide && !self.toolbar_revealed && self.toolbar_animation.is_none();
        let state = serde_json::json!({
            "autoHide": auto_hide,
            "collapsed": collapsed,
            "pinned": self.preferences.settings.toolbar_pinned,
            "fullscreen": self.window.fullscreen().is_some(),
        });
        let _ = self
            .toolbar
            .evaluate_script(&format!("window.cometToolbar?.layout({state})"));
    }
    fn advance_toolbar(&mut self, now: Instant) -> bool {
        if let Some(animation) = &self.toolbar_animation {
            let (height, finished) = animation.sample(now);
            self.toolbar_height = height;
            if finished {
                self.toolbar_animation = None;
            }
            return finished;
        }
        false
    }
    /// Advance from MainEventsCleared without creating a separate animation task.
    pub(super) fn animate_toolbar(&mut self) {
        if self.toolbar_animation.is_none() {
            return;
        }
        let finished = self.advance_toolbar(Instant::now());
        if finished {
            self.resize();
        } else {
            self.apply_toolbar_bounds();
        }
    }
    /// Keep hidden and visible body views aligned for a seamless later switch.
    fn apply_toolbar_bounds(&self) {
        let (bar, body) = ui::layout(&self.window, self.toolbar_height);
        let _ = self.toolbar.set_bounds(bar);
        let _ = self.manager.set_bounds(body);
        if let Some(viewer) = &self.remote {
            let _ = viewer.view.set_bounds(body);
        }
    }
}

/// Respect Windows' client-area animation setting; retain the default on failure.
/// The HTML/CSS interfaces separately honor `prefers-reduced-motion`.
pub(super) fn animations_enabled() -> bool {
    use windows::{
        Win32::UI::WindowsAndMessaging::{
            SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        },
        core::BOOL,
    };
    let mut enabled = BOOL(1);
    // SPI_GETCLIENTAREAANIMATION writes a BOOL into the live, correctly sized value.
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&mut enabled as *mut BOOL).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    enabled.as_bool()
}

#[cfg(test)]
mod toolbar_animation_tests {
    use super::*;
    #[test]
    fn animation_reaches_both_ends_and_reversal_starts_at_current_height() {
        let start = Instant::now();
        let hide = ToolbarAnimation {
            from: 48.,
            to: 3.,
            started: start,
        };
        assert_eq!(hide.sample(start), (48., false));
        let midway = hide.sample(start + Duration::from_millis(80)).0;
        assert!(midway > 3. && midway < 48.);
        assert_eq!(hide.sample(start + TOOLBAR_ANIMATION), (3., true));
        let reveal = ToolbarAnimation {
            from: midway,
            to: 48.,
            started: start,
        };
        assert_eq!(reveal.sample(start), (midway, false));
        assert_eq!(reveal.sample(start + TOOLBAR_ANIMATION), (48., true));
    }
}
