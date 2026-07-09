use std::collections::HashMap;
use std::path::PathBuf;

use niri_ipc::PickedColor;
use zbus::fdo::{self, RequestNameFlags};
use zbus::zvariant::OwnedValue;
use zbus::{interface, zvariant};

use super::Start;

pub struct Screenshot {
    to_niri: calloop::channel::Sender<ScreenshotToNiri>,
    from_niri: async_channel::Receiver<NiriToScreenshot>,
}

pub enum ScreenshotToNiri {
    TakeScreenshot { include_cursor: bool },
    PickColor(async_channel::Sender<Option<PickedColor>>),
}

pub enum NiriToScreenshot {
    ScreenshotResult(Option<PathBuf>),
}

#[interface(name = "org.gnome.Shell.Screenshot")]
impl Screenshot {
    async fn screenshot(
        &self,
        include_cursor: bool,
        _flash: bool,
        _filename: PathBuf,
    ) -> fdo::Result<(bool, PathBuf)> {
        if let Err(err) = self
            .to_niri
            .send(ScreenshotToNiri::TakeScreenshot { include_cursor })
        {
            warn!("error sending message to niri: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        let filename = match self.from_niri.recv().await {
            Ok(NiriToScreenshot::ScreenshotResult(Some(filename))) => filename,
            Ok(NiriToScreenshot::ScreenshotResult(None)) => {
                return Err(fdo::Error::Failed("internal error".to_owned()));
            }
            Err(err) => {
                warn!("error receiving message from niri: {err:?}");
                return Err(fdo::Error::Failed("internal error".to_owned()));
            }
        };

        Ok((true, filename))
    }

    async fn pick_color(&self) -> fdo::Result<HashMap<String, OwnedValue>> {
        let (tx, rx) = async_channel::bounded(1);
        if let Err(err) = self.to_niri.send(ScreenshotToNiri::PickColor(tx)) {
            warn!("error sending pick color message to niri: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        let color = match rx.recv().await {
            Ok(Some(color)) => color,
            Ok(None) => {
                return Err(fdo::Error::Failed("no color picked".to_owned()));
            }
            Err(err) => {
                warn!("error receiving message from niri: {err:?}");
                return Err(fdo::Error::Failed("internal error".to_owned()));
            }
        };

        let mut result = HashMap::new();
        let [r, g, b] = color.rgb;
        result.insert(
            "color".to_string(),
            zvariant::OwnedValue::try_from(zvariant::Value::from((r, g, b))).unwrap(),
        );

        Ok(result)
    }
}

impl Screenshot {
    pub fn new(
        to_niri: calloop::channel::Sender<ScreenshotToNiri>,
        from_niri: async_channel::Receiver<NiriToScreenshot>,
    ) -> Self {
        Self { to_niri, from_niri }
    }
}

impl Start for Screenshot {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::Connection::session()?;
        let flags = RequestNameFlags::AllowReplacement
            | RequestNameFlags::ReplaceExisting
            | RequestNameFlags::DoNotQueue;

        conn.object_server()
            .at("/org/gnome/Shell/Screenshot", self)?;
        conn.request_name_with_flags("org.gnome.Shell.Screenshot", flags)?;

        Ok(conn)
    }
}
