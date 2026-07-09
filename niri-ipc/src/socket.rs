//! Helper for blocking communication over the niri socket.

use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;

use crate::{Event, Reply, Request};

/// Name of the environment variable containing the niri IPC socket path.
pub const SOCKET_PATH_ENV: &str = "NIRI_SOCKET";

/// Helper for blocking communication over the niri socket.
///
/// This struct is used to communicate with the niri IPC server. It handles the socket connection
/// and serialization/deserialization of messages.
pub struct Socket {
    stream: BufReader<UnixStream>,
}

impl Socket {
    /// Connects to the default niri IPC socket.
    ///
    /// This is equivalent to calling [`Self::connect_to`] with the path taken from the
    /// [`SOCKET_PATH_ENV`] environment variable.
    pub fn connect() -> io::Result<Self> {
        let socket_path = env::var_os(SOCKET_PATH_ENV).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{SOCKET_PATH_ENV} is not set, are you running this within niri?"),
            )
        })?;
        Self::connect_to(socket_path)
    }

    /// Connects to the niri IPC socket at the given path.
    pub fn connect_to(path: impl AsRef<Path>) -> io::Result<Self> {
        let stream = UnixStream::connect(path.as_ref())?;
        let stream = BufReader::new(stream);
        Ok(Self { stream })
    }

    /// Sends a request to niri and returns the response.
    ///
    /// Return values:
    ///
    /// * `Ok(Ok(response))`: successful [`Response`](crate::Response) from niri
    /// * `Ok(Err(message))`: error message from niri
    /// * `Err(error)`: error communicating with niri
    pub fn send(&mut self, request: Request) -> io::Result<Reply> {
        let mut buf = serde_json::to_string(&request).unwrap();
        buf.push('\n');
        self.stream.get_mut().write_all(buf.as_bytes())?;

        buf.clear();
        self.stream.read_line(&mut buf)?;

        let reply = serde_json::from_str(&buf)?;
        Ok(reply)
    }

    /// Starts reading event stream [`Event`]s from the socket.
    ///
    /// The returned function will block until the next [`Event`] arrives, then return it.
    ///
    /// Use this only after requesting an [`EventStream`][Request::EventStream].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use niri_ipc::{Request, Response};
    /// use niri_ipc::socket::Socket;
    ///
    /// fn main() -> std::io::Result<()> {
    ///     let mut socket = Socket::connect()?;
    ///
    ///     let reply = socket.send(Request::EventStream)?;
    ///     if matches!(reply, Ok(Response::Handled)) {
    ///         let mut read_event = socket.read_events();
    ///         while let Ok(event) = read_event() {
    ///             println!("Received event: {event:?}");
    ///         }
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn read_events(self) -> impl FnMut() -> io::Result<Event> {
        let Self { mut stream } = self;
        let _ = stream.get_mut().shutdown(Shutdown::Write);

        let mut buf = String::new();
        move || {
            buf.clear();
            stream.read_line(&mut buf)?;
            let event = serde_json::from_str(&buf)?;
            Ok(event)
        }
    }
}
