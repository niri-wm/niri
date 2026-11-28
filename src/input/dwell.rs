use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use calloop::timer::{TimeoutAction, Timer};
use calloop::RegistrationToken;
use niri_config::DwellClick;
use niri_ipc::{DwellClickMode, DwellClickState};
use smithay::backend::input::{ButtonState, InputTime, MouseButton};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::Uniform;
use smithay::output::Output;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::niri::{OutputRenderElements, State};
use crate::render_helpers::renderer::NiriRenderer;
use crate::render_helpers::shader_element::ShaderRenderElement;
use crate::render_helpers::shaders::{ProgramType, Shaders};

pub(crate) const BTN_LEFT: u32 = 0x110;
pub(crate) const BTN_RIGHT: u32 = 0x111;
pub(crate) const BTN_MIDDLE: u32 = 0x112;

pub(crate) const CLICK_FEEDBACK_DURATION: Duration = Duration::from_millis(100);
pub(crate) const DOUBLE_CLICK_TIMESTAMP_OFFSET_MICROS: u64 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DwellPhase {
    Idle,
    Armed,
    Delaying,
    Dwelling,
    ClickFeedback,
}

impl DwellPhase {
    fn has_timer(self) -> bool {
        matches!(self, Self::Delaying | Self::Dwelling | Self::ClickFeedback)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DwellClickAction {
    Single(MouseButton, u32),
    DoubleClick,
    Drag(ButtonState),
}

#[derive(Debug)]
pub struct DwellClicker {
    config: DwellClick,
    enabled_override: Option<bool>,
    mode: DwellClickMode,
    mode_locked: bool,
    dragging: bool,
    phase: DwellPhase,
    anchor: Option<Point<f64, Logical>>,
    stage_started_at: Option<Duration>,
    timer_seq: u64,
    timer: Option<RegistrationToken>,
}

impl DwellClicker {
    pub fn new(config: DwellClick) -> Self {
        Self {
            mode: config.default_mode,
            mode_locked: config.mode_locked,
            config,
            enabled_override: None,
            dragging: false,
            phase: DwellPhase::Idle,
            anchor: None,
            stage_started_at: None,
            timer_seq: 0,
            timer: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled_override.unwrap_or(self.config.on)
    }

    pub fn is_active(&self) -> bool {
        matches!(self.phase, DwellPhase::Delaying | DwellPhase::Dwelling)
    }

    pub fn is_idle(&self) -> bool {
        self.phase == DwellPhase::Idle && self.anchor.is_none()
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn state(&self) -> DwellClickState {
        DwellClickState {
            enabled: self.is_enabled(),
            mode: self.mode,
            default_mode: self.config.default_mode,
            mode_locked: self.mode_locked,
            dragging: self.dragging,
        }
    }

    pub fn are_animations_ongoing(&self) -> bool {
        self.config.indicator.on
            && self.is_enabled()
            && matches!(self.phase, DwellPhase::Dwelling | DwellPhase::ClickFeedback)
    }

    fn timer_duration(&self) -> Option<Duration> {
        match self.phase {
            DwellPhase::Delaying => Some(Duration::from_millis(u64::from(self.config.delay_ms))),
            DwellPhase::Dwelling => Some(Duration::from_millis(u64::from(
                self.config.dwell_ms.max(1),
            ))),
            DwellPhase::ClickFeedback => Some(CLICK_FEEDBACK_DURATION),
            DwellPhase::Idle | DwellPhase::Armed => None,
        }
    }

    pub(crate) fn update_config(&mut self, config: DwellClick) {
        let old_config = self.config;
        let was_enabled = self.is_enabled();

        self.config = config;

        if config.on != old_config.on {
            self.enabled_override = None;
        }
        if config.default_mode != old_config.default_mode && !self.dragging {
            self.mode = config.default_mode;
        }
        if config.mode_locked != old_config.mode_locked {
            self.mode_locked = config.mode_locked;
        }

        if was_enabled && !self.is_enabled() {
            self.reset_all();
            return;
        }
        if config.delay_ms != old_config.delay_ms
            || config.dwell_ms != old_config.dwell_ms
            || config.motion_threshold != old_config.motion_threshold
        {
            self.reset_pending();
        }
    }

    pub(crate) fn toggle_enabled(&mut self) {
        let target = !self.is_enabled();
        self.enabled_override = Some(target);

        if target {
            self.reset_pending();
        } else {
            self.reset_all();
        }
    }

    pub(crate) fn set_mode(&mut self, mode: DwellClickMode) {
        if self.mode == mode && !self.dragging {
            return;
        }

        self.reset_pending();
        self.clear_drag();
        self.mode = mode;
    }

    pub(crate) fn toggle_lock(&mut self) {
        self.mode_locked = !self.mode_locked;
    }

    pub(crate) fn cancel_drag(&mut self) {
        if self.dragging {
            self.reset_all();
        }
    }

    pub(crate) fn on_motion(&mut self, pos: Point<f64, Logical>, now: Duration) {
        if !self.is_enabled() {
            return;
        }

        let Some(anchor) = self.anchor else {
            self.anchor = Some(pos);
            if self.phase == DwellPhase::Idle {
                self.phase = DwellPhase::Armed;
            }
            return;
        };

        if !exceeds_threshold(anchor, pos, self.config.motion_threshold) {
            return;
        }

        self.anchor = Some(pos);
        let phase = if self.config.delay_ms > 0 {
            DwellPhase::Delaying
        } else {
            DwellPhase::Dwelling
        };
        self.enter_timed_phase(phase, now);
    }

    pub(crate) fn on_timer(&mut self, now: Duration) -> Option<DwellClickAction> {
        if !self.is_enabled() {
            return None;
        }

        match self.phase {
            DwellPhase::Delaying => {
                self.enter_timed_phase(DwellPhase::Dwelling, now);
                None
            }
            DwellPhase::Dwelling => Some(self.trigger_click(now)),
            DwellPhase::ClickFeedback => {
                self.phase = if self.anchor.is_some() {
                    DwellPhase::Armed
                } else {
                    DwellPhase::Idle
                };
                self.stage_started_at = None;
                None
            }
            DwellPhase::Idle | DwellPhase::Armed => None,
        }
    }

    pub(crate) fn on_physical_button(&mut self, button: Option<MouseButton>, state: ButtonState) {
        self.reset_pending();
        if button == Some(MouseButton::Left) && state == ButtonState::Released {
            self.clear_drag();
        }
    }

    fn reset_mode_if_unlocked(&mut self) {
        if !self.mode_locked {
            self.mode = self.config.default_mode;
        }
    }

    fn clear_drag(&mut self) {
        if self.dragging {
            self.dragging = false;
            self.reset_mode_if_unlocked();
        }
    }

    fn enter_timed_phase(&mut self, phase: DwellPhase, now: Duration) {
        self.phase = phase;
        self.timer_seq = self.timer_seq.wrapping_add(1);
        self.stage_started_at =
            matches!(phase, DwellPhase::Dwelling | DwellPhase::ClickFeedback).then_some(now);
    }

    pub(crate) fn reset_pending(&mut self) {
        self.anchor = None;
        self.phase = DwellPhase::Idle;
        self.stage_started_at = None;
    }

    pub(crate) fn reset_all(&mut self) {
        self.reset_pending();
        self.clear_drag();
    }

    fn trigger_click(&mut self, now: Duration) -> DwellClickAction {
        let action = match self.mode {
            DwellClickMode::Left => DwellClickAction::Single(MouseButton::Left, BTN_LEFT),
            DwellClickMode::Right => DwellClickAction::Single(MouseButton::Right, BTN_RIGHT),
            DwellClickMode::Middle => DwellClickAction::Single(MouseButton::Middle, BTN_MIDDLE),
            DwellClickMode::Double => DwellClickAction::DoubleClick,
            DwellClickMode::Drag => {
                if self.dragging {
                    self.dragging = false;
                    DwellClickAction::Drag(ButtonState::Released)
                } else {
                    self.dragging = true;
                    DwellClickAction::Drag(ButtonState::Pressed)
                }
            }
        };

        // Require pointer movement beyond the threshold before starting the next dwell.
        self.anchor = None;

        if !self.dragging {
            self.reset_mode_if_unlocked();
        }

        self.enter_timed_phase(DwellPhase::ClickFeedback, now);

        action
    }

    /// Returns `(dwell_progress, clicked_progress)` in `0.0..=1.0`, or `None` when hidden.
    pub fn visual_state(&self, now: Duration) -> Option<(f32, f32)> {
        if !self.config.indicator.on || !self.is_enabled() {
            return None;
        }

        let started = self.stage_started_at?;
        match self.phase {
            DwellPhase::Dwelling => {
                let total = Duration::from_millis(u64::from(self.config.dwell_ms.max(1)));
                let progress =
                    (now.saturating_sub(started).as_secs_f64() / total.as_secs_f64()).clamp(0., 1.);
                Some((progress as f32, 0.))
            }
            DwellPhase::ClickFeedback => {
                let progress = (now.saturating_sub(started).as_secs_f64()
                    / CLICK_FEEDBACK_DURATION.as_secs_f64())
                .clamp(0., 1.);
                Some((1., progress as f32))
            }
            DwellPhase::Idle | DwellPhase::Armed | DwellPhase::Delaying => None,
        }
    }

    pub fn render<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        output: &Output,
        output_geo: Rectangle<f64, Logical>,
        pointer_pos: Point<f64, Logical>,
        xcursor_size: u8,
        now: Duration,
        push: &mut dyn FnMut(OutputRenderElements<R>),
    ) {
        let Some((dwell_progress, clicked_progress)) = self.visual_state(now) else {
            return;
        };

        if Shaders::get(renderer)
            .program(ProgramType::DwellIndicator)
            .is_none()
        {
            return;
        }

        let indicator = self.config.indicator;
        let radius = indicator
            .radius
            .unwrap_or_else(|| f64::from(xcursor_size) * 0.75)
            .max(4.);
        let width = indicator.width.clamp(1., radius);
        let pad = 2.0;
        let half_box = radius + pad;
        let box_size = half_box * 2.0;

        let local_center = pointer_pos - output_geo.loc;
        let area = Rectangle::new(
            local_center - Point::from((half_box, half_box)),
            Size::from((box_size, box_size)),
        );
        if !area.overlaps(Rectangle::from_size(output_geo.size)) {
            return;
        }

        let scale = output.current_scale().fractional_scale() as f32;
        let uniforms = Rc::from([
            Uniform::new("color", indicator.color.to_array_premul()),
            Uniform::new("dwell_progress", dwell_progress),
            Uniform::new("clicked_progress", clicked_progress),
            Uniform::new("stroke_width", width as f32),
            Uniform::new("outer_radius", radius as f32),
        ]);

        let elem = ShaderRenderElement::new(
            ProgramType::DwellIndicator,
            area.size,
            None,
            scale,
            1.0,
            uniforms,
            HashMap::new(),
            Kind::Unspecified,
        )
        .with_location(area.loc);
        push(OutputRenderElements::Shader(elem));
    }
}

impl State {
    pub(crate) fn dwell_update_config(&mut self, config: DwellClick) {
        self.with_dwell(|dc| dc.update_config(config));
    }

    pub(crate) fn dwell_toggle_enabled(&mut self) {
        self.with_dwell(DwellClicker::toggle_enabled);
    }

    pub(crate) fn dwell_set_mode(&mut self, mode: DwellClickMode) {
        self.with_dwell(|dc| dc.set_mode(mode));
    }

    pub(crate) fn dwell_toggle_lock(&mut self) {
        self.with_dwell(DwellClicker::toggle_lock);
    }

    pub(crate) fn dwell_cancel_drag(&mut self) {
        self.with_dwell(DwellClicker::cancel_drag);
    }

    pub(crate) fn dwell_on_motion(&mut self, pos: Point<f64, Logical>) {
        if !self.niri.dwell_click.is_enabled() {
            return;
        }
        let now = self.niri.clock.now_unadjusted();
        self.with_dwell(|dc| dc.on_motion(pos, now));
    }

    pub(crate) fn dwell_on_physical_button(
        &mut self,
        button: Option<MouseButton>,
        state: ButtonState,
    ) {
        if self.niri.dwell_click.is_idle() && !self.niri.dwell_click.is_dragging() {
            return;
        }
        self.dwell_apply(false, |dc| {
            dc.on_physical_button(button, state);
            None
        });
    }

    pub(crate) fn dwell_cancel_pending(&mut self) {
        if self.niri.dwell_click.is_idle() {
            return;
        }
        self.with_dwell(DwellClicker::reset_pending);
    }

    pub(crate) fn dwell_cancel_all(&mut self) {
        if self.niri.dwell_click.is_idle() && !self.niri.dwell_click.is_dragging() {
            return;
        }
        self.with_dwell(DwellClicker::reset_all);
    }

    fn dwell_on_timer(&mut self) {
        let now = self.niri.clock.now_unadjusted();
        self.dwell_apply(false, |dc| dc.on_timer(now));
    }

    fn with_dwell(&mut self, f: impl FnOnce(&mut DwellClicker)) {
        self.dwell_apply(true, |dc| {
            f(dc);
            None
        });
    }

    fn dwell_apply(
        &mut self,
        release_cancelled_drag: bool,
        f: impl FnOnce(&mut DwellClicker) -> Option<DwellClickAction>,
    ) {
        let dc = &mut self.niri.dwell_click;
        let prev_state = dc.state();
        let prev_phase = dc.phase;
        let prev_seq = dc.timer_seq;
        let prev_visual = dc.are_animations_ongoing();
        let prev_dragging = dc.dragging;

        let click = f(dc);
        let cancel_drag = release_cancelled_drag && prev_dragging && !dc.dragging;
        let timer_changed = dc.timer_seq != prev_seq;
        let timer_cancelled = prev_phase.has_timer() && !dc.phase.has_timer();

        if timer_changed || timer_cancelled {
            if let Some(token) = dc.timer.take() {
                self.niri.event_loop.remove(token);
            }
        }
        if timer_changed {
            if let Some(duration) = dc.timer_duration() {
                let timer = Timer::from_duration(duration);
                let token = self
                    .niri
                    .event_loop
                    .insert_source(timer, |_, _, state| {
                        state.niri.dwell_click.timer = None;
                        state.dwell_on_timer();
                        TimeoutAction::Drop
                    })
                    .unwrap();
                self.niri.dwell_click.timer = Some(token);
            }
        }

        let dc = &self.niri.dwell_click;
        let cur_visual = dc.are_animations_ongoing();
        let state_differs = dc.state() != prev_state;
        if prev_visual != cur_visual || (cur_visual && (dc.phase != prev_phase || timer_changed)) {
            self.niri.queue_redraw_all();
        }

        if cancel_drag {
            self.cancel_dwell_drag_grab();
        }
        if let Some(action) = click {
            self.execute_dwell_click(action);
        }
        if state_differs {
            self.ipc_refresh_dwell_click();
        }
    }

    fn prepare_synthetic_pointer(&mut self) -> InputTime {
        self.niri.advance_animations();
        if self.niri.monitors_active {
            self.niri.notify_activity();
        }
        self.niri.pointer_inactivity_timer_got_reset = false;
        self.niri.reset_pointer_inactivity_timer();
        InputTime::now()
    }

    fn dismiss_overlays_for_click(&mut self) {
        if self.niri.hotkey_overlay.hide() {
            self.niri.queue_redraw_all();
        }
        if self.niri.exit_confirm_dialog.hide() {
            self.niri.queue_redraw_all();
        }
    }

    fn execute_dwell_click(&mut self, action: DwellClickAction) {
        let _span = tracy_client::span!("State::execute_dwell_click");

        let now = self.prepare_synthetic_pointer();
        match action {
            DwellClickAction::Single(button, code) => {
                self.handle_pointer_button(Some(button), code, ButtonState::Pressed, now);
                self.handle_pointer_button(Some(button), code, ButtonState::Released, now);
            }
            DwellClickAction::DoubleClick => {
                let second_time = InputTime::from_micros(
                    now.micros()
                        .saturating_add(DOUBLE_CLICK_TIMESTAMP_OFFSET_MICROS),
                );
                for time in [now, second_time] {
                    self.handle_pointer_button(
                        Some(MouseButton::Left),
                        BTN_LEFT,
                        ButtonState::Pressed,
                        time,
                    );
                    self.handle_pointer_button(
                        Some(MouseButton::Left),
                        BTN_LEFT,
                        ButtonState::Released,
                        time,
                    );
                }
            }
            DwellClickAction::Drag(state) => {
                self.handle_pointer_button(Some(MouseButton::Left), BTN_LEFT, state, now);
            }
        }
        self.dismiss_overlays_for_click();
    }

    fn cancel_dwell_drag_grab(&mut self) {
        let now = self.prepare_synthetic_pointer();
        self.handle_pointer_button(
            Some(MouseButton::Left),
            BTN_LEFT,
            ButtonState::Released,
            now,
        );
    }
}

fn exceeds_threshold(
    anchor: Point<f64, Logical>,
    pos: Point<f64, Logical>,
    threshold: f64,
) -> bool {
    let dx = pos.x - anchor.x;
    let dy = pos.y - anchor.y;
    dx * dx + dy * dy >= threshold * threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_config() -> DwellClick {
        DwellClick {
            on: true,
            ..DwellClick::default()
        }
    }

    fn pt(x: f64, y: f64) -> Point<f64, Logical> {
        Point::from((x, y))
    }

    #[test]
    fn disabled_by_default_ignores_motion() {
        let mut dc = DwellClicker::new(DwellClick::default());
        assert!(!dc.is_enabled());
        dc.on_motion(pt(10., 10.), Duration::ZERO);
        dc.on_motion(pt(100., 100.), Duration::ZERO);
        assert_eq!(dc.phase, DwellPhase::Idle);
    }

    #[test]
    fn requires_movement_beyond_threshold_to_arm_and_ignores_jitter() {
        let mut dc = DwellClicker::new(enabled_config());
        let t = Duration::ZERO;

        dc.on_motion(pt(100., 100.), t);
        assert_eq!(dc.phase, DwellPhase::Armed);

        dc.on_motion(pt(103., 103.), t);
        assert_eq!(dc.phase, DwellPhase::Armed);

        // Fixed anchor prevents slow drift from triggering below radius 5.
        dc.on_motion(pt(103.5, 103.5), t);
        assert_eq!(dc.phase, DwellPhase::Armed);

        dc.on_motion(pt(104., 103.), t);
        assert_eq!(dc.phase, DwellPhase::Delaying);

        dc.on_motion(pt(106., 105.), t);
        assert_eq!(dc.phase, DwellPhase::Delaying);

        assert_eq!(dc.on_timer(t), None);
        assert_eq!(dc.phase, DwellPhase::Dwelling);

        assert_eq!(
            dc.on_timer(t),
            Some(DwellClickAction::Single(MouseButton::Left, BTN_LEFT))
        );
        assert_eq!(dc.phase, DwellPhase::ClickFeedback);

        assert_eq!(dc.on_timer(t), None);
        assert_eq!(dc.phase, DwellPhase::Idle);

        dc.on_motion(pt(106., 105.), t);
        dc.on_motion(pt(108., 106.), t);
        assert_eq!(dc.phase, DwellPhase::Armed);
    }

    #[test]
    fn euclidean_circle_rejects_diagonal_at_box_corner() {
        let mut dc = DwellClicker::new(enabled_config());
        let t = Duration::ZERO;
        dc.on_motion(pt(0., 0.), t);

        // (3.5, 3.5) is inside L_2=5 circle (24.5 < 25); (4.9, 4.9) is inside L_inf=5 but >= 25.
        dc.on_motion(pt(3.5, 3.5), t);
        assert_eq!(dc.phase, DwellPhase::Armed);
        dc.on_motion(pt(4.9, 4.9), t);
        assert_eq!(dc.phase, DwellPhase::Delaying);
    }

    #[test]
    fn one_shot_mode_resets_to_default_unless_locked() {
        let mut dc = DwellClicker::new(enabled_config());
        let t = Duration::ZERO;

        dc.set_mode(DwellClickMode::Right);
        assert_eq!(dc.state().mode, DwellClickMode::Right);

        dc.on_motion(pt(0., 0.), t);
        dc.on_motion(pt(10., 0.), t);
        dc.on_timer(t);
        assert_eq!(
            dc.on_timer(t),
            Some(DwellClickAction::Single(MouseButton::Right, BTN_RIGHT))
        );
        assert_eq!(dc.state().mode, DwellClickMode::Left);

        dc.toggle_lock();
        dc.set_mode(DwellClickMode::Middle);
        dc.on_motion(pt(0., 0.), t);
        dc.on_motion(pt(10., 0.), t);
        dc.on_timer(t);
        assert_eq!(
            dc.on_timer(t),
            Some(DwellClickAction::Single(MouseButton::Middle, BTN_MIDDLE))
        );
        assert_eq!(dc.state().mode, DwellClickMode::Middle);
    }

    #[test]
    fn two_dwell_drag_and_cancel_mid_drag() {
        let mut dc = DwellClicker::new(enabled_config());
        let t = Duration::ZERO;
        dc.set_mode(DwellClickMode::Drag);

        dc.on_motion(pt(0., 0.), t);
        dc.on_motion(pt(10., 0.), t);
        dc.on_timer(t);
        assert_eq!(
            dc.on_timer(t),
            Some(DwellClickAction::Drag(ButtonState::Pressed))
        );
        assert!(dc.is_dragging());
        assert_eq!(dc.state().mode, DwellClickMode::Drag);

        dc.on_motion(pt(50., 50.), t);
        dc.on_motion(pt(60., 50.), t);
        dc.on_timer(t);
        assert_eq!(
            dc.on_timer(t),
            Some(DwellClickAction::Drag(ButtonState::Released))
        );
        assert!(!dc.is_dragging());
        assert_eq!(dc.state().mode, DwellClickMode::Left);

        dc.set_mode(DwellClickMode::Drag);
        dc.on_motion(pt(0., 0.), t);
        dc.on_motion(pt(10., 0.), t);
        dc.on_timer(t);
        dc.on_timer(t);
        assert!(dc.is_dragging());

        dc.set_mode(DwellClickMode::Left);
        assert!(!dc.is_dragging());
    }

    #[test]
    fn physical_click_cancels_pending_and_syncs_drag() {
        let mut dc = DwellClicker::new(enabled_config());
        let t = Duration::ZERO;
        dc.on_motion(pt(0., 0.), t);
        dc.on_motion(pt(10., 0.), t);
        assert_eq!(dc.phase, DwellPhase::Delaying);

        dc.on_physical_button(Some(MouseButton::Left), ButtonState::Pressed);
        assert_eq!(dc.phase, DwellPhase::Idle);

        dc.set_mode(DwellClickMode::Drag);
        dc.on_motion(pt(0., 0.), t);
        dc.on_motion(pt(10., 0.), t);
        dc.on_timer(t);
        dc.on_timer(t);
        assert!(dc.is_dragging());

        dc.on_physical_button(Some(MouseButton::Left), ButtonState::Released);
        assert!(!dc.is_dragging());
        assert_eq!(dc.state().mode, DwellClickMode::Left);
    }

    #[test]
    fn config_reload_overrides_runtime_toggle_only_when_on_changes() {
        let mut dc = DwellClicker::new(DwellClick::default());
        assert!(!dc.is_enabled());

        dc.toggle_enabled();
        assert!(dc.is_enabled());

        let mut cfg = DwellClick {
            dwell_ms: 400,
            ..DwellClick::default()
        };
        dc.update_config(cfg);
        assert!(dc.is_enabled());

        cfg.on = true;
        dc.update_config(cfg);
        assert!(dc.is_enabled());

        cfg.on = false;
        dc.update_config(cfg);
        assert!(!dc.is_enabled());
    }

    #[test]
    fn visual_state_progress_restart_and_off_flag() {
        let mut cfg = enabled_config();
        cfg.delay_ms = 0;
        cfg.dwell_ms = 500;
        let mut dc = DwellClicker::new(cfg);

        let t0 = Duration::from_secs(10);
        assert_eq!(dc.visual_state(t0), None);
        assert!(!dc.are_animations_ongoing());

        dc.on_motion(pt(0., 0.), t0);
        dc.on_motion(pt(10., 0.), t0);
        assert!(dc.are_animations_ongoing());
        assert_eq!(dc.visual_state(t0), Some((0.0, 0.0)));
        assert_eq!(
            dc.visual_state(t0 + Duration::from_millis(250)),
            Some((0.5, 0.0))
        );

        // Jitter inside threshold must not reset timer_seq or stage_started_at.
        let t_mid = t0 + Duration::from_millis(250);
        let seq_before = dc.timer_seq;
        dc.on_motion(pt(11., 0.), t_mid);
        assert_eq!(dc.timer_seq, seq_before);
        assert_eq!(dc.visual_state(t_mid), Some((0.5, 0.0)));

        // Restarting directly in Dwelling (delay_ms = 0) bumps timer_seq and resets timestamp.
        dc.on_motion(pt(30., 0.), t_mid);
        assert_ne!(dc.timer_seq, seq_before);
        assert_eq!(dc.visual_state(t_mid), Some((0.0, 0.0)));

        let t1 = t_mid + Duration::from_millis(500);
        dc.on_timer(t1);
        assert!(dc.are_animations_ongoing());
        assert_eq!(
            dc.visual_state(t1 + Duration::from_millis(50)),
            Some((1.0, 0.5))
        );

        cfg.indicator.on = false;
        dc.update_config(cfg);
        assert!(!dc.are_animations_ongoing());
        assert_eq!(dc.visual_state(t1 + Duration::from_millis(50)), None);
    }
}
