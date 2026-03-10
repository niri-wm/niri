use std::collections::HashMap;

use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeTuple;
use serde::{Deserialize, Serialize};
use zbus::fdo::{self, RequestNameFlags};
use zbus::interface;
use zbus::zvariant::{SerializeDict, Type, Value};

use super::Start;

// Gnome portal converts modifiers into this format and
// back again afterwards. Their settings provider seems to use a different format
// than everything else and their portal expects that.
const GNOME_PORTAL_KEY_MAP: &[(&str, &str)] = &[
    ("<ctrl>", "Ctrl"),
    ("<shift>", "Shift"),
    ("<alt>", "Alt"),
    ("<mod2>", "Num_Lock"),
    ("<super>", "Super"),
];

pub struct ShortcutsProvider {
    to_niri: calloop::channel::Sender<ShortcutsProviderToNiri>,
}

pub enum ShortcutsProviderToNiri {
    BindShortcuts {
        app_id: String,
        shortcuts: Vec<BindShortcutRequest>,
        results: async_channel::Sender<Vec<BindShortcutResponse>>,
    },
}

#[derive(Debug, Type, Clone)]
#[zvariant(signature = "sa{sv}")]
pub struct BindShortcutRequest {
    pub id: String,
    pub description: String,
    pub preferred_trigger: Vec<String>,
}

#[derive(Debug, Type, Clone)]
#[zvariant(signature = "sa{sv}")]
pub struct BindShortcutResponse {
    pub id: String,
    pub description: String,
    pub shortcuts: Vec<String>,
}

#[interface(name = "org.gnome.Settings.GlobalShortcutsProvider")]
impl ShortcutsProvider {
    async fn bind_shortcuts(
        &self,
        app_id: String,
        _parent_window: String,
        shortcuts: Vec<BindShortcutRequest>,
    ) -> fdo::Result<Vec<BindShortcutResponse>> {
        let (tx, rx) = async_channel::bounded(1);

        if let Err(err) = self.to_niri.send(ShortcutsProviderToNiri::BindShortcuts {
            app_id,
            shortcuts,
            results: tx,
        }) {
            warn!("error sending bind shortcuts message to niri: {err:?}");
            return Err(fdo::Error::Failed("internal error".to_owned()));
        }

        rx.recv().await.map_err(|err| {
            warn!("error receiving message from niri: {err:?}");
            fdo::Error::Failed("internal error".to_owned())
        })
    }
}

impl ShortcutsProvider {
    pub fn new(to_niri: calloop::channel::Sender<ShortcutsProviderToNiri>) -> Self {
        Self { to_niri }
    }
}

impl<'de> Deserialize<'de> for BindShortcutRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct OptsVisitor;

        impl<'de> Visitor<'de> for OptsVisitor {
            type Value = BindShortcutRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str(
                    "a tuple of (string, array of (string, string|array of string) pairs)",
                )
            }

            fn visit_seq<V>(self, mut seq: V) -> Result<BindShortcutRequest, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let id = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::missing_field("id"))?;

                let opts: HashMap<String, Value> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::missing_field("options"))?;

                let description = opts
                    .get("description")
                    .and_then(|v| match v {
                        Value::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_default()
                    .to_string();

                // preferred_trigger is a Value wrapping either `s` or `as`
                let preferred_trigger = if let Some(v) = opts.get("preferred_trigger").cloned() {
                    match v {
                        Value::Str(s) => vec![s.to_string()],
                        Value::Array(arr) => {
                            let mut str_vec: Vec<String> = Vec::new();
                            for elem in arr.iter() {
                                match elem.downcast_ref::<String>() {
                                    Ok(s) => str_vec.push(s),
                                    Err(_) => return Err(de::Error::custom("Vardict entry `preferred_trigger` contained array of a variant *other* than string")),
                                };
                            }
                            str_vec
                        }
                        _ => {
                            return Err(de::Error::custom(
                                "Vardict entry `preferred_trigger` was neither string nor array",
                            ))
                        }
                    }
                } else {
                    Vec::new()
                };

                // Convert from gnome's formatting `<...>` tags around modifier keys
                let formatted_trigger = preferred_trigger
                    .into_iter()
                    .map(|trigger| convert_gnome_modifiers(&trigger))
                    .collect();

                Ok(BindShortcutRequest {
                    id,
                    description,
                    preferred_trigger: formatted_trigger,
                })
            }
        }

        deserializer.deserialize_tuple(2, OptsVisitor)
    }
}

impl Serialize for BindShortcutResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let opts = BindShortcutResponseOpts {
            description: self.description.clone(),
            shortcuts: self.shortcuts.clone(),
        };

        let mut tuple = serializer.serialize_tuple(2)?;
        tuple.serialize_element(&self.id)?;
        tuple.serialize_element(&opts)?;
        tuple.end()
    }
}

// Helper struct to serialize into dbus dictionaries correctly
#[derive(SerializeDict, Debug, Clone)]
#[zvariant(signature = "dict")]
struct BindShortcutResponseOpts {
    description: String,
    shortcuts: Vec<String>,
}

impl Start for ShortcutsProvider {
    fn start(self) -> anyhow::Result<zbus::blocking::Connection> {
        let conn = zbus::blocking::Connection::session()?;
        let flags = RequestNameFlags::AllowReplacement
            | RequestNameFlags::ReplaceExisting
            | RequestNameFlags::DoNotQueue;

        conn.object_server()
            .at("/org/gnome/Settings/GlobalShortcutsProvider", self)?;
        conn.request_name_with_flags("org.gnome.Settings.GlobalShortcutsProvider", flags)?;

        Ok(conn)
    }
}

fn convert_gnome_modifiers(input: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let mut remaining = input;

    while let Some(start) = remaining.find('<') {
        if let Some(end) = remaining[start..].find('>') {
            let token = &remaining[start..=start + end];
            if let Some(&(_, mapped)) = GNOME_PORTAL_KEY_MAP
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(token))
            {
                parts.push(mapped);
            }
            remaining = &remaining[start + end + 1..];
        } else {
            break;
        }
    }

    if !remaining.is_empty() {
        parts.push(remaining);
    }

    parts.join("+")
}
