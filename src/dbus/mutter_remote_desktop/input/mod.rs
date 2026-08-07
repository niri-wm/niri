mod device;
mod events;

use smithay::backend::input::{AxisSource, ButtonState, InputEvent, KeyState, Keycode};

use self::device::{RemoteDesktopEventBase, RemoteDesktopInputBackend};
use self::events::{
    RemoteDesktopKeyboardEvent, RemoteDesktopPointerAxisEvent, RemoteDesktopPointerButtonEvent,
    RemoteDesktopPointerMotionAbsoluteEvent, RemoteDesktopPointerMotionEvent,
    RemoteDesktopTouchFrameEvent, RemoteDesktopTouchPositionEvent, RemoteDesktopTouchSlotEvent,
};
use super::clipboard::{
    disable_clipboard, enable_clipboard, read_clipboard, set_selection,
    RemoteDesktopClipboardReadReply, RemoteDesktopClipboardReply,
};
use super::eis::RemoteDesktopEisConnection;
use super::keymap::{
    apply_keymap_change, set_layout_index, RemoteDesktopKeymapChange, RemoteDesktopKeymapReply,
};
use crate::niri::State;
use crate::utils::get_monotonic_time;

#[derive(Debug)]
pub enum RemoteDesktopToNiri {
    ConnectEis(RemoteDesktopEisConnection),
    SetKeymap {
        change: RemoteDesktopKeymapChange,
        reply: RemoteDesktopKeymapReply,
    },
    SetKeymapLayoutIndex {
        index: u32,
        reply: RemoteDesktopKeymapReply,
    },
    EnableClipboard {
        session_id: String,
        mime_types: Option<Vec<String>>,
        reply: RemoteDesktopClipboardReply,
    },
    DisableClipboard {
        session_id: String,
        reply: RemoteDesktopClipboardReply,
    },
    SetSelection {
        session_id: String,
        mime_types: Option<Vec<String>>,
        reply: RemoteDesktopClipboardReply,
    },
    ReadClipboard {
        session_id: String,
        mime_type: String,
        reply: RemoteDesktopClipboardReadReply,
    },
    KeyboardKeycode {
        keycode: Keycode,
        state: KeyState,
    },
    PointerButton {
        button: u32,
        state: ButtonState,
    },
    PointerAxis {
        frame: RemoteDesktopAxisFrame,
    },
    PointerMotionRelative {
        dx: f64,
        dy: f64,
    },
    PointerMotionAbsolute {
        position: RemoteDesktopPosition,
    },
    TouchDown {
        slot: u32,
        position: RemoteDesktopPosition,
    },
    TouchMotion {
        slot: u32,
        position: RemoteDesktopPosition,
    },
    TouchUp {
        slot: u32,
    },
}

#[derive(Debug, Clone, Copy)]
pub struct RemoteDesktopAxisFrame {
    pub smooth: Option<(f64, f64)>,
    pub v120: Option<(f64, f64)>,
    pub source: AxisSource,
    pub stop: (bool, bool),
}

#[derive(Debug, Clone)]
pub struct RemoteDesktopPosition {
    pub output_name: String,
    pub x: f64,
    pub y: f64,
    pub width: i32,
    pub height: i32,
}

pub fn dispatch_to_niri(state: &mut State, msg: RemoteDesktopToNiri) {
    let time_msec = event_time_msec();

    match msg {
        RemoteDesktopToNiri::ConnectEis(connection) => {
            if let Err(err) = super::eis::connect_to_niri(state, connection) {
                warn!("error connecting remote desktop EIS client: {err}");
            }
        }
        RemoteDesktopToNiri::SetKeymap { change, reply } => {
            let _ = reply.try_send(apply_keymap_change(state, change));
        }
        RemoteDesktopToNiri::SetKeymapLayoutIndex { index, reply } => {
            let _ = reply.try_send(set_layout_index(state, index));
        }
        RemoteDesktopToNiri::EnableClipboard {
            session_id,
            mime_types,
            reply,
        } => {
            let _ = reply.try_send(enable_clipboard(state, session_id, mime_types));
        }
        RemoteDesktopToNiri::DisableClipboard { session_id, reply } => {
            let _ = reply.try_send(disable_clipboard(state, session_id));
        }
        RemoteDesktopToNiri::SetSelection {
            session_id,
            mime_types,
            reply,
        } => {
            let _ = reply.try_send(set_selection(state, session_id, mime_types));
        }
        RemoteDesktopToNiri::ReadClipboard {
            session_id,
            mime_type,
            reply,
        } => {
            let _ = reply.try_send(read_clipboard(state, session_id, mime_type));
        }
        RemoteDesktopToNiri::KeyboardKeycode {
            keycode,
            state: key_state,
        } => {
            state.process_input_event(InputEvent::<RemoteDesktopInputBackend>::Keyboard {
                event: RemoteDesktopKeyboardEvent {
                    base: RemoteDesktopEventBase::keyboard(time_msec),
                    keycode,
                    state: key_state,
                },
            });
        }
        RemoteDesktopToNiri::PointerButton {
            button,
            state: button_state,
        } => {
            state.process_input_event(InputEvent::<RemoteDesktopInputBackend>::PointerButton {
                event: RemoteDesktopPointerButtonEvent {
                    base: RemoteDesktopEventBase::pointer(time_msec, None),
                    button,
                    state: button_state,
                },
            });
        }
        RemoteDesktopToNiri::PointerAxis { frame } => {
            state.process_input_event(InputEvent::<RemoteDesktopInputBackend>::PointerAxis {
                event: RemoteDesktopPointerAxisEvent {
                    base: RemoteDesktopEventBase::pointer(time_msec, None),
                    frame,
                },
            });
        }
        RemoteDesktopToNiri::PointerMotionRelative { dx, dy } => {
            state.process_input_event(InputEvent::<RemoteDesktopInputBackend>::PointerMotion {
                event: RemoteDesktopPointerMotionEvent {
                    base: RemoteDesktopEventBase::pointer(time_msec, None),
                    dx,
                    dy,
                },
            });
        }
        RemoteDesktopToNiri::PointerMotionAbsolute { position } => {
            let output_name = Some(position.output_name.clone());
            state.process_input_event(
                InputEvent::<RemoteDesktopInputBackend>::PointerMotionAbsolute {
                    event: RemoteDesktopPointerMotionAbsoluteEvent {
                        base: RemoteDesktopEventBase::pointer(time_msec, output_name),
                        position,
                    },
                },
            );
        }
        RemoteDesktopToNiri::TouchDown { slot, position } => {
            dispatch_touch_position(
                state,
                timed_touch(time_msec, slot, position),
                TouchPhase::Down,
            );
        }
        RemoteDesktopToNiri::TouchMotion { slot, position } => {
            dispatch_touch_position(
                state,
                timed_touch(time_msec, slot, position),
                TouchPhase::Motion,
            );
        }
        RemoteDesktopToNiri::TouchUp { slot } => {
            state.process_input_event(InputEvent::<RemoteDesktopInputBackend>::TouchUp {
                event: RemoteDesktopTouchSlotEvent {
                    base: RemoteDesktopEventBase::touch(time_msec, None),
                    slot,
                },
            });
            dispatch_touch_frame(state, time_msec);
        }
    }
}

enum TouchPhase {
    Down,
    Motion,
}

struct TimedTouch {
    time_msec: u32,
    slot: u32,
    position: RemoteDesktopPosition,
}

fn timed_touch(time_msec: u32, slot: u32, position: RemoteDesktopPosition) -> TimedTouch {
    TimedTouch {
        time_msec,
        slot,
        position,
    }
}

fn dispatch_touch_position(state: &mut State, touch: TimedTouch, phase: TouchPhase) {
    let output_name = Some(touch.position.output_name.clone());
    let event = RemoteDesktopTouchPositionEvent {
        base: RemoteDesktopEventBase::touch(touch.time_msec, output_name),
        slot: touch.slot,
        position: touch.position,
    };

    match phase {
        TouchPhase::Down => {
            state.process_input_event(InputEvent::<RemoteDesktopInputBackend>::TouchDown { event });
        }
        TouchPhase::Motion => {
            state.process_input_event(InputEvent::<RemoteDesktopInputBackend>::TouchMotion {
                event,
            });
        }
    }

    dispatch_touch_frame(state, touch.time_msec);
}

fn dispatch_touch_frame(state: &mut State, time_msec: u32) {
    state.process_input_event(InputEvent::<RemoteDesktopInputBackend>::TouchFrame {
        event: RemoteDesktopTouchFrameEvent {
            base: RemoteDesktopEventBase::touch(time_msec, None),
        },
    });
}

fn event_time_msec() -> u32 {
    u32::try_from(get_monotonic_time().as_millis()).unwrap_or(u32::MAX)
}
