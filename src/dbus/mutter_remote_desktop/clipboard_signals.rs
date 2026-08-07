use std::collections::HashMap;
use std::fs::File;
use std::os::fd::OwnedFd as StdOwnedFd;
use std::thread;

use zbus::object_server::SignalEmitter;
use zbus::zvariant::Value;

use super::session::Session;

pub(super) fn emit_owner_changed(
    signal_emitter: SignalEmitter<'static>,
    session_is_owner: bool,
    mime_types: Vec<String>,
) {
    let task_name = "emit remote desktop clipboard owner changed";
    let connection = signal_emitter.connection().clone();
    connection
        .executor()
        .spawn(
            async move {
                let mut options = HashMap::new();
                options.insert("mime-types", Value::new(mime_types));
                options.insert("session-is-owner", Value::new(session_is_owner));

                if let Err(err) = Session::selection_owner_changed(&signal_emitter, options).await {
                    warn!("error emitting RemoteDesktop SelectionOwnerChanged signal: {err:?}");
                }
            },
            task_name,
        )
        .detach();
}

pub(super) fn emit_selection_transfer(
    signal_emitter: SignalEmitter<'static>,
    mime_type: String,
    serial: u32,
) {
    let task_name = "emit remote desktop clipboard transfer";
    let connection = signal_emitter.connection().clone();
    connection
        .executor()
        .spawn(
            async move {
                if let Err(err) =
                    Session::selection_transfer(&signal_emitter, &mime_type, serial).await
                {
                    warn!("error emitting RemoteDesktop SelectionTransfer signal: {err:?}");
                }
            },
            task_name,
        )
        .detach();
}

pub(super) fn copy_fd(source_fd: StdOwnedFd, target_fd: StdOwnedFd) {
    thread::spawn(move || {
        let mut source = File::from(source_fd);
        let mut target = File::from(target_fd);
        if let Err(err) = std::io::copy(&mut source, &mut target) {
            warn!("error copying remote desktop clipboard transfer: {err:?}");
        }
    });
}
