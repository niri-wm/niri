use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use wayland_backend::server::ClientId;

use super::raw::hyprland_lock_notify::v1::server::hyprland_lock_notification_v1::{
    self, HyprlandLockNotificationV1,
};
use super::raw::hyprland_lock_notify::v1::server::hyprland_lock_notifier_v1::{
    self, HyprlandLockNotifierV1,
};

const VERSION: u32 = 1;

pub struct HyprlandLockNotifyManagerState {
    notifications: Vec<HyprlandLockNotificationV1>,
}

impl HyprlandLockNotifyManagerState {
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<HyprlandLockNotifierV1, ()>,
        D: Dispatch<HyprlandLockNotifierV1, ()>,
        D: Dispatch<HyprlandLockNotificationV1, ()>,
        D: 'static,
    {
        display.create_global::<D, HyprlandLockNotifierV1, _>(VERSION, ());
        Self {
            notifications: Vec::new(),
        }
    }

    pub fn send_locked(&self) {
        for notification in &self.notifications {
            notification.locked();
        }
    }

    pub fn send_unlocked(&self) {
        for notification in &self.notifications {
            notification.unlocked();
        }
    }
}

pub trait HyprlandLockNotifyHandler {
    fn lock_notify_state(&mut self) -> &mut HyprlandLockNotifyManagerState;
    fn is_locked(&self) -> bool;
}

// Implement GlobalDispatch for HyprlandLockNotifierV1
impl<D> GlobalDispatch<HyprlandLockNotifierV1, (), D> for HyprlandLockNotifyManagerState
where
    D: GlobalDispatch<HyprlandLockNotifierV1, ()>,
    D: Dispatch<HyprlandLockNotifierV1, ()>,
    D: Dispatch<HyprlandLockNotificationV1, ()>,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<HyprlandLockNotifierV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(resource, ());
    }
}

// Implement Dispatch for HyprlandLockNotifierV1
impl<D> Dispatch<HyprlandLockNotifierV1, (), D> for HyprlandLockNotifyManagerState
where
    D: Dispatch<HyprlandLockNotifierV1, ()>,
    D: Dispatch<HyprlandLockNotificationV1, ()>,
    D: HyprlandLockNotifyHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &HyprlandLockNotifierV1,
        request: <HyprlandLockNotifierV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            hyprland_lock_notifier_v1::Request::Destroy => {
                // No specific cleanup needed
            }
            hyprland_lock_notifier_v1::Request::GetLockNotification { id } => {
                let notification = data_init.init(id, ());

                // If currently locked, send locked event immediately
                if state.is_locked() {
                    notification.locked();
                }

                state.lock_notify_state().notifications.push(notification);
            }
        }
    }
}

// Implement Dispatch for HyprlandLockNotificationV1
impl<D> Dispatch<HyprlandLockNotificationV1, (), D> for HyprlandLockNotifyManagerState
where
    D: Dispatch<HyprlandLockNotificationV1, ()>,
    D: HyprlandLockNotifyHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &HyprlandLockNotificationV1,
        request: <HyprlandLockNotificationV1 as Resource>::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            hyprland_lock_notification_v1::Request::Destroy => {
                state
                    .lock_notify_state()
                    .notifications
                    .retain(|n| n != resource);
            }
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        resource: &HyprlandLockNotificationV1,
        _data: &(),
    ) {
        state
            .lock_notify_state()
            .notifications
            .retain(|n| n != resource);
    }
}

#[macro_export]
macro_rules! delegate_hyprland_lock_notify {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::hyprland_lock_notify::v1::server::hyprland_lock_notifier_v1::HyprlandLockNotifierV1: ()
        ] => $crate::protocols::hyprland_lock_notify::HyprlandLockNotifyManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::hyprland_lock_notify::v1::server::hyprland_lock_notifier_v1::HyprlandLockNotifierV1: ()
        ] => $crate::protocols::hyprland_lock_notify::HyprlandLockNotifyManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::hyprland_lock_notify::v1::server::hyprland_lock_notification_v1::HyprlandLockNotificationV1: ()
        ] => $crate::protocols::hyprland_lock_notify::HyprlandLockNotifyManagerState);
    };
}
