//! The `org.freedesktop.Notifications` D-Bus interface.
//!
//! zbus runs the interface on its own executor thread; everything it receives
//! is forwarded to the thread that draws through a calloop channel, so the UI
//! never has to be thread-safe.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use smithay_client_toolkit::reexports::calloop::channel::Sender;
use zbus::zvariant::OwnedValue;

use crate::model::{resolve_timeout, strip_markup, Id, IdAllocator, Notification, Urgency};

/// Object path and interface name, both fixed by the specification.
pub const PATH: &str = "/org/freedesktop/Notifications";
pub const INTERFACE: &str = "org.freedesktop.Notifications";
pub const BUS_NAME: &str = "org.freedesktop.Notifications";

/// Why a notification went away, as the spec numbers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    Requested = 3,
}

/// What the D-Bus thread sends to the drawing thread.
#[derive(Debug)]
pub enum Message {
    Show { notification: Notification, replaces_id: Id },
    Close(Id),
}

/// The interface implementation.
pub struct Service {
    tx: Sender<Message>,
    ids: Arc<IdAllocator>,
}

impl Service {
    pub fn new(tx: Sender<Message>, ids: Arc<IdAllocator>) -> Self {
        Self { tx, ids }
    }
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Service {
    /// Show a notification. Returns the id the caller can later close.
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let urgency = urgency_from_hints(&hints);
        let now = Instant::now();
        // The id is allocated here because `Notify` has to answer immediately,
        // before the drawing thread has even seen the notification.
        let id = self.ids.next();

        let notification = Notification {
            id,
            app_name: fallback(app_name, "Notification"),
            summary: strip_markup(&summary),
            body: strip_markup(&body),
            urgency,
            expires_at: resolve_timeout(expire_timeout, urgency, now),
        };

        if self.tx.send(Message::Show { notification, replaces_id }).is_err() {
            tracing::warn!("the notification surface is gone; dropping a notification");
        }
        // A replacement keeps the id the sender already knows.
        if replaces_id != 0 {
            replaces_id
        } else {
            id
        }
    }

    fn close_notification(&self, id: u32) {
        let _ = self.tx.send(Message::Close(id));
    }

    /// What this server supports. Actions and icons are not implemented yet, so
    /// they are deliberately not claimed: a sender that is told a capability
    /// exists will rely on it.
    fn get_capabilities(&self) -> Vec<String> {
        vec![String::from("body"), String::from("body-markup"), String::from("persistence")]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            String::from("spectre-notify"),
            String::from("Spectre DE"),
            String::from(env!("CARGO_PKG_VERSION")),
            String::from("1.2"),
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;
}

/// Read the `urgency` hint, whatever integer type the sender used for it.
fn urgency_from_hints(hints: &HashMap<String, OwnedValue>) -> Urgency {
    let Some(value) = hints.get("urgency") else {
        return Urgency::Normal;
    };
    // Senders are inconsistent about the type here; the spec says byte, but
    // uint32 and int32 both turn up in the wild.
    let raw = u8::try_from(value)
        .ok()
        .or_else(|| u32::try_from(value).ok().map(|v| v.min(255) as u8))
        .or_else(|| i32::try_from(value).ok().map(|v| v.clamp(0, 255) as u8));
    raw.map(Urgency::from_hint).unwrap_or(Urgency::Normal)
}

fn fallback(value: String, default: &str) -> String {
    if value.trim().is_empty() {
        default.to_owned()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    fn hints(value: Value<'static>) -> HashMap<String, OwnedValue> {
        let mut map = HashMap::new();
        map.insert(String::from("urgency"), OwnedValue::try_from(value).unwrap());
        map
    }

    #[test]
    fn a_missing_urgency_hint_means_normal() {
        assert_eq!(urgency_from_hints(&HashMap::new()), Urgency::Normal);
    }

    #[test]
    fn the_urgency_hint_is_read_whatever_integer_type_it_arrives_as() {
        // The spec says byte; real senders use uint32 and int32 too.
        assert_eq!(urgency_from_hints(&hints(Value::U8(2))), Urgency::Critical);
        assert_eq!(urgency_from_hints(&hints(Value::U32(2))), Urgency::Critical);
        assert_eq!(urgency_from_hints(&hints(Value::I32(0))), Urgency::Low);
    }

    #[test]
    fn a_nonsense_urgency_hint_falls_back_to_normal() {
        assert_eq!(urgency_from_hints(&hints(Value::Str("high".into()))), Urgency::Normal);
        assert_eq!(urgency_from_hints(&hints(Value::I32(-5))), Urgency::Low);
    }

    #[test]
    fn an_empty_application_name_gets_a_placeholder() {
        assert_eq!(fallback(String::new(), "Notification"), "Notification");
        assert_eq!(fallback(String::from("  "), "Notification"), "Notification");
        assert_eq!(fallback(String::from("Firefox"), "Notification"), "Firefox");
    }

    #[test]
    fn close_reasons_match_the_specification() {
        assert_eq!(CloseReason::Expired as u32, 1);
        assert_eq!(CloseReason::Dismissed as u32, 2);
        assert_eq!(CloseReason::Requested as u32, 3);
    }
}
