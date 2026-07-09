//! Swipe gesture from scroll events.
//!
//! Tracks when to begin, update, and end a swipe gesture from pointer axis events, also whether
//! the gesture is vertical or horizontal. Necessary because libinput only provides touchpad swipe
//! gesture events for 3+ fingers.

#[derive(Debug)]
pub struct ScrollSwipeGesture {
    ongoing: bool,
    vertical: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    BeginUpdate,
    Update,
    End,
}

impl ScrollSwipeGesture {
    pub const fn new() -> Self {
        Self {
            ongoing: false,
            vertical: false,
        }
    }

    pub fn update(&mut self, dx: f64, dy: f64) -> Action {
        if dx == 0. && dy == 0. {
            self.ongoing = false;
            Action::End
        } else if !self.ongoing {
            self.ongoing = true;
            self.vertical = dy != 0.;
            Action::BeginUpdate
        } else {
            Action::Update
        }
    }

    pub fn reset(&mut self) -> bool {
        if self.ongoing {
            self.ongoing = false;
            true
        } else {
            false
        }
    }

    pub fn is_vertical(&self) -> bool {
        self.vertical
    }
}

impl Default for ScrollSwipeGesture {
    fn default() -> Self {
        Self::new()
    }
}

impl Action {
    pub fn begin(self) -> bool {
        self == Action::BeginUpdate
    }

    pub fn end(self) -> bool {
        self == Action::End
    }
}
