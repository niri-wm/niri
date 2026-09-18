use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use atomic::Ordering;
use calloop::ping::{make_ping, Ping};
use calloop::timer::{TimeoutAction, Timer};
use calloop::LoopHandle;
use smithay::reexports::wayland_server::Client;
use smithay::wayland::compositor::{Blocker, BlockerState};

/// Default time limit, after which the transaction completes.
///
/// Serves to avoid hanging when a client fails to respond to a configure promptly.
const TIME_LIMIT: Duration = Duration::from_millis(300);

/// Transaction between Wayland clients.
///
/// How to use it:
/// 1. Create a transaction with [`Transaction::new()`].
/// 2. Clone it as many times as you need.
/// 3. Before adding the transaction as a commit blocker, remember to call
///    [`Transaction::add_notification()`] to receive a notification when the transaction completes.
/// 4. Before adding the transaction as a commit blocker, remember to call
///    [`Transaction::register_deadline_timer()`] to make sure the transaction completes when
///    reaching the deadline.
/// 5. In your surface pre-commit handler, if the transaction corresponding to that commit isn't
///    ready, get a blocker with [`Transaction::blocker()`] and add it to the surface.
#[derive(Debug, Clone)]
pub struct Transaction {
    inner: Arc<Inner>,
    deadline: Rc<RefCell<Deadline>>,
}

/// Blocker for a [`Transaction`].
#[derive(Debug)]
pub struct TransactionBlocker(Weak<Inner>);

#[derive(Debug)]
enum Deadline {
    NotRegistered(Instant),
    Registered { remove: Ping },
}

#[derive(Debug)]
struct Inner {
    /// Whether the transaction is completed.
    completed: AtomicBool,
    /// Notifications to send out upon completing the transaction.
    notifications: Mutex<Option<(Sender<Client>, Vec<Client>)>>,
}

impl Transaction {
    /// Creates a new transaction.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner::new()),
            deadline: Rc::new(RefCell::new(Deadline::NotRegistered(
                Instant::now() + TIME_LIMIT,
            ))),
        }
    }

    /// Gets a blocker for this transaction.
    pub fn blocker(&self) -> TransactionBlocker {
        trace!(transaction = ?Arc::as_ptr(&self.inner), "generating blocker");
        TransactionBlocker(Arc::downgrade(&self.inner))
    }

    /// Adds a notification for when this transaction completes.
    pub fn add_notification(&self, sender: Sender<Client>, client: Client) {
        if self.is_completed() {
            error!("tried to add notification to a completed transaction");
            return;
        }

        let mut guard = self.inner.notifications.lock().unwrap();
        guard.get_or_insert((sender, Vec::new())).1.push(client);
    }

    /// Registers this transaction's deadline timer on an event loop.
    pub fn register_deadline_timer<T: 'static>(&self, event_loop: &LoopHandle<'static, T>) {
        let mut cell = self.deadline.borrow_mut();
        if let Deadline::NotRegistered(deadline) = *cell {
            let timer = Timer::from_deadline(deadline);
            let inner = Arc::downgrade(&self.inner);

            let token = match event_loop.insert_source(timer, move |_, _, _| {
                let _span = trace_span!("deadline timer", transaction = ?Weak::as_ptr(&inner))
                    .entered();
                #[cfg(not(test))]
                if let Some(inner) = inner.upgrade() {
                    trace!("deadline reached, completing transaction");
                    inner.complete();
                } else {
                    trace!("transaction completed without removing the timer");
                }
                TimeoutAction::Drop
            }) {
                Ok(token) => token,
                Err(err) => {
                    error!("failed to register transaction deadline timer: {err}");
                    return;
                }
            };

            let (ping, source) = make_ping().unwrap();
            let loop_handle = event_loop.clone();
            if let Err(err) = event_loop.insert_source(source, move |_, _, _| {
                loop_handle.remove(token);
            }) {
                // Rollback: Remove timer to prevent a leak
                event_loop.remove(token);
                error!("failed to register transaction ping source: {err}");
                return;
            }

            *cell = Deadline::Registered { remove: ping };
        }
    }

    /// Returns whether this transaction has already completed.
    pub fn is_completed(&self) -> bool {
        self.inner.is_completed()
    }

    /// Returns whether this is the last instance of this transaction.
    pub fn is_last(&self) -> bool {
        Arc::strong_count(&self.inner) == 1
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        let _span = trace_span!("drop", transaction = ?Arc::as_ptr(&self.inner)).entered();

        if self.is_last() {
            // If this was the last transaction, complete it.
            trace!("last transaction dropped, completing");
            self.inner.complete();

            // Also remove the timer.
            if let Deadline::Registered { remove } = &*self.deadline.borrow() {
                remove.ping();
            };
        }
    }
}

impl TransactionBlocker {
    pub fn completed() -> Self {
        Self(Weak::new())
    }
}

impl Blocker for TransactionBlocker {
    fn state(&self) -> BlockerState {
        if self.0.upgrade().is_none_or(|x| x.is_completed()) {
            BlockerState::Released
        } else {
            BlockerState::Pending
        }
    }
}

impl Inner {
    fn new() -> Self {
        Self {
            completed: AtomicBool::new(false),
            notifications: Mutex::new(None),
        }
    }

    fn is_completed(&self) -> bool {
        self.completed.load(Ordering::Relaxed)
    }

    fn complete(&self) {
        self.completed.store(true, Ordering::Relaxed);

        let mut guard = self.notifications.lock().unwrap();
        if let Some((sender, clients)) = guard.take() {
            for client in clients {
                if let Err(err) = sender.send(client) {
                    warn!("error sending blocker notification: {err:?}");
                };
            }
        }
    }
}
