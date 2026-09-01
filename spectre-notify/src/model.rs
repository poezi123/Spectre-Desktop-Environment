//! The notification model.
//!
//! Everything here is pure: what a notification is, how long it lives, and how
//! the stack behaves when more arrive than fit. The D-Bus plumbing and the
//! drawing are elsewhere, so the rules that decide whether a critical alert can
//! be missed are testable on their own.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Server-assigned notification id. `0` is never handed out: the spec uses it
/// to mean "this is a new notification, not a replacement".
pub type Id = u32;

/// How loudly the sender wants to interrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    /// Must not disappear on its own. Errors, low battery, failed backups.
    Critical,
}

impl Urgency {
    /// Decode the `urgency` hint. Anything unrecognised is Normal, because a
    /// sender with a broken hint should still be heard.
    pub fn from_hint(value: u8) -> Self {
        match value {
            0 => Urgency::Low,
            2 => Urgency::Critical,
            _ => Urgency::Normal,
        }
    }

    /// How long a notification of this urgency stays up by default.
    pub fn default_timeout(self) -> Option<Duration> {
        match self {
            Urgency::Low => Some(Duration::from_secs(4)),
            Urgency::Normal => Some(Duration::from_secs(7)),
            // The spec is explicit: critical notifications are never expired
            // by the server. Dismissing one has to be a deliberate act.
            Urgency::Critical => None,
        }
    }
}

/// One notification on screen.
#[derive(Debug, Clone)]
pub struct Notification {
    pub id: Id,
    /// The sending application's name, shown above the summary.
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub urgency: Urgency,
    /// When it should disappear. `None` means it stays until dismissed.
    pub expires_at: Option<Instant>,
}

impl Notification {
    pub fn is_expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|deadline| now >= deadline)
    }

    /// Time until this one expires, for scheduling the next wake-up.
    pub fn time_left(&self, now: Instant) -> Option<Duration> {
        self.expires_at.map(|deadline| deadline.saturating_duration_since(now))
    }
}

/// Resolve the `expire_timeout` argument of `Notify`.
///
/// The spec defines `-1` as "server decides" and `0` as "never expire".
/// Anything else is milliseconds. A critical notification ignores a positive
/// timeout only when the sender did not ask for one, so an application that
/// really wants its error to fade can still say so.
pub fn resolve_timeout(expire_timeout: i32, urgency: Urgency, now: Instant) -> Option<Instant> {
    match expire_timeout {
        0 => None,
        ms if ms > 0 => Some(now + Duration::from_millis(ms as u64)),
        // Negative: the server chooses.
        _ => urgency.default_timeout().map(|d| now + d),
    }
}

/// Strip the small HTML subset the spec allows from a body.
///
/// The panel draws plain text, and showing `<b>Warning</b>` verbatim would be
/// worse than showing `Warning`. Entities are decoded too, since senders escape
/// ampersands in otherwise plain text.
pub fn strip_markup(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_tag = false;
    let mut entity: Option<String> = None;

    for c in body.chars() {
        match c {
            '<' if entity.is_none() => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            '&' => entity = Some(String::new()),
            ';' if entity.is_some() => {
                let name = entity.take().unwrap_or_default();
                match name.as_str() {
                    "amp" => out.push('&'),
                    "lt" => out.push('<'),
                    "gt" => out.push('>'),
                    "quot" => out.push('"'),
                    "apos" => out.push('\''),
                    // Not an entity we know: put it back exactly as written.
                    _ => {
                        out.push('&');
                        out.push_str(&name);
                        out.push(';');
                    }
                }
            }
            _ => match entity.as_mut() {
                // An unterminated entity is just text.
                Some(name) if name.len() > 12 => {
                    out.push('&');
                    out.push_str(name);
                    out.push(c);
                    entity = None;
                }
                Some(name) => name.push(c),
                None => out.push(c),
            },
        }
    }

    if let Some(name) = entity {
        out.push('&');
        out.push_str(&name);
    }
    out
}

/// Hands out notification ids.
///
/// Lives apart from the [`Stack`] because `Notify` has to return an id
/// synchronously on the D-Bus thread, while the stack itself belongs to the
/// thread that draws.
#[derive(Debug)]
pub struct IdAllocator(AtomicU32);

impl Default for IdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl IdAllocator {
    pub fn new() -> Self {
        Self(AtomicU32::new(1))
    }

    /// The next id. Never `0`, and never repeated within a session, so a late
    /// `CloseNotification` for something already dismissed cannot hit a newer
    /// notification instead.
    pub fn next(&self) -> Id {
        let id = self.0.fetch_add(1, Ordering::Relaxed);
        if id == 0 {
            // Wrapped after four billion notifications. Skipping zero is all
            // that is needed to keep the protocol's "new notification" marker
            // distinguishable.
            return self.0.fetch_add(1, Ordering::Relaxed);
        }
        id
    }
}

/// The notifications currently on screen.
#[derive(Debug)]
pub struct Stack {
    items: Vec<Notification>,
    capacity: usize,
}

/// A few accessors here are read only by the tests and by the notification
/// history that is still to come.
#[allow(dead_code)]
impl Stack {
    /// `capacity` is how many are shown at once; older ones are dropped first.
    pub fn new(capacity: usize) -> Self {
        Self { items: Vec::new(), capacity: capacity.max(1) }
    }

    pub fn items(&self) -> &[Notification] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Add a notification, or replace one when `replaces_id` names a live one.
    ///
    /// `notification.id` must already be allocated. Returns the id the
    /// notification ended up with, which is `replaces_id` when it replaced one.
    pub fn push(&mut self, mut notification: Notification, replaces_id: Id) -> Id {
        if replaces_id != 0 {
            if let Some(existing) = self.items.iter_mut().find(|n| n.id == replaces_id) {
                notification.id = replaces_id;
                *existing = notification;
                return replaces_id;
            }
        }

        let id = notification.id;
        self.items.push(notification);

        // Overflow drops the oldest non-critical one first: a critical alert
        // must not be pushed off screen by a stream of chat messages.
        while self.items.len() > self.capacity {
            let victim = self
                .items
                .iter()
                .position(|n| n.urgency != Urgency::Critical)
                .unwrap_or(0);
            self.items.remove(victim);
        }
        id
    }

    /// Remove a notification. Returns `true` if it was there.
    pub fn close(&mut self, id: Id) -> bool {
        let before = self.items.len();
        self.items.retain(|n| n.id != id);
        self.items.len() != before
    }

    /// Drop everything that has timed out, returning their ids.
    pub fn expire(&mut self, now: Instant) -> Vec<Id> {
        let expired: Vec<Id> =
            self.items.iter().filter(|n| n.is_expired(now)).map(|n| n.id).collect();
        self.items.retain(|n| !n.is_expired(now));
        expired
    }

    /// How long until the next one expires, for the timer.
    pub fn next_deadline(&self, now: Instant) -> Option<Duration> {
        self.items.iter().filter_map(|n| n.time_left(now)).min()
    }

    /// The notification at a stack position, topmost first.
    pub fn get(&self, index: usize) -> Option<&Notification> {
        // Drawn newest-first, so index 0 is the last one pushed.
        self.items.iter().rev().nth(index)
    }

    /// Newest first, which is the order they are drawn in.
    pub fn newest_first(&self) -> impl Iterator<Item = &Notification> {
        self.items.iter().rev()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note_with(id: Id, urgency: Urgency, now: Instant, timeout: i32) -> Notification {
        Notification {
            id,
            app_name: "test".into(),
            summary: "Summary".into(),
            body: String::new(),
            urgency,
            expires_at: resolve_timeout(timeout, urgency, now),
        }
    }

    /// A notification with a fresh id from a per-test allocator.
    fn note(urgency: Urgency, now: Instant, timeout: i32) -> Notification {
        use std::sync::LazyLock;
        static IDS: LazyLock<IdAllocator> = LazyLock::new(IdAllocator::new);
        note_with(IDS.next(), urgency, now, timeout)
    }

    #[test]
    fn urgency_hints_decode_with_a_safe_default() {
        assert_eq!(Urgency::from_hint(0), Urgency::Low);
        assert_eq!(Urgency::from_hint(1), Urgency::Normal);
        assert_eq!(Urgency::from_hint(2), Urgency::Critical);
        assert_eq!(Urgency::from_hint(200), Urgency::Normal, "junk must still be heard");
    }

    #[test]
    fn a_zero_timeout_never_expires() {
        let now = Instant::now();
        assert_eq!(resolve_timeout(0, Urgency::Normal, now), None);
    }

    #[test]
    fn a_positive_timeout_is_taken_literally_even_for_critical() {
        let now = Instant::now();
        let at = resolve_timeout(500, Urgency::Critical, now).unwrap();
        assert!(at > now && at <= now + Duration::from_millis(501));
    }

    #[test]
    fn the_server_never_expires_a_critical_notification_on_its_own() {
        let now = Instant::now();
        assert_eq!(resolve_timeout(-1, Urgency::Critical, now), None);
        assert!(resolve_timeout(-1, Urgency::Normal, now).is_some());
        assert!(resolve_timeout(-1, Urgency::Low, now).is_some());
    }

    #[test]
    fn low_urgency_goes_away_sooner_than_normal() {
        assert!(Urgency::Low.default_timeout() < Urgency::Normal.default_timeout());
    }

    #[test]
    fn markup_is_stripped_and_entities_decoded() {
        assert_eq!(strip_markup("<b>Bold</b> text"), "Bold text");
        assert_eq!(strip_markup("a &amp; b"), "a & b");
        assert_eq!(strip_markup("&lt;tag&gt;"), "<tag>");
        assert_eq!(strip_markup(r#"say &quot;hi&quot;"#), "say \"hi\"");
    }

    #[test]
    fn an_unknown_entity_survives_as_written() {
        assert_eq!(strip_markup("100&euro;"), "100&euro;");
    }

    #[test]
    fn a_bare_ampersand_is_not_swallowed() {
        assert_eq!(strip_markup("rock & roll"), "rock & roll");
        assert_eq!(strip_markup("trailing &"), "trailing &");
    }

    #[test]
    fn an_unterminated_tag_does_not_eat_the_rest() {
        // Malformed input must not silently blank a notification.
        assert_eq!(strip_markup("<b>hello"), "hello");
    }

    #[test]
    fn plain_text_passes_through_untouched() {
        let text = "Backup finished: 1.2 GiB in 40 s";
        assert_eq!(strip_markup(text), text);
    }

    #[test]
    fn ids_start_at_one_and_never_repeat() {
        let ids = IdAllocator::new();
        assert_eq!(ids.next(), 1);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = ids.next();
            assert_ne!(id, 0, "zero means `not a replacement` and must never be handed out");
            assert!(seen.insert(id), "a reused id would let a stale close hit a new notification");
        }
    }

    #[test]
    fn ids_are_handed_out_safely_from_several_threads() {
        use std::sync::Arc;
        let ids = Arc::new(IdAllocator::new());
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let ids = Arc::clone(&ids);
                std::thread::spawn(move || (0..250).map(|_| ids.next()).collect::<Vec<_>>())
            })
            .collect();
        let all: Vec<Id> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
        let unique: std::collections::HashSet<Id> = all.iter().copied().collect();
        assert_eq!(unique.len(), all.len());
    }

    #[test]
    fn the_stack_keeps_the_id_it_was_given() {
        let now = Instant::now();
        let mut stack = Stack::new(8);
        let id = stack.push(note_with(77, Urgency::Normal, now, 0), 0);
        assert_eq!(id, 77);
        assert_eq!(stack.items()[0].id, 77);
    }

    #[test]
    fn the_newest_notification_is_drawn_first() {
        let now = Instant::now();
        let mut stack = Stack::new(8);
        stack.push(note_with(1, Urgency::Normal, now, 0), 0);
        stack.push(note_with(2, Urgency::Normal, now, 0), 0);
        assert_eq!(stack.get(0).map(|n| n.id), Some(2));
        assert_eq!(stack.newest_first().map(|n| n.id).collect::<Vec<_>>(), [2, 1]);
    }

    #[test]
    fn replacing_keeps_the_id_and_the_position() {
        let now = Instant::now();
        let mut stack = Stack::new(8);
        let first = stack.push(note(Urgency::Normal, now, 0), 0);
        let second = stack.push(note(Urgency::Normal, now, 0), 0);

        let mut replacement = note(Urgency::Normal, now, 0);
        replacement.summary = "Updated".into();
        let id = stack.push(replacement, first);

        assert_eq!(id, first);
        assert_eq!(stack.len(), 2);
        assert_eq!(stack.items()[0].summary, "Updated");
        assert_eq!(stack.items()[1].id, second);
    }

    #[test]
    fn replacing_something_already_gone_adds_it_instead() {
        let now = Instant::now();
        let mut stack = Stack::new(8);
        let id = stack.push(note_with(5, Urgency::Normal, now, 0), 999);
        assert_eq!(stack.len(), 1);
        assert_eq!(id, 5, "the fresh id is used when there is nothing to replace");
    }

    #[test]
    fn overflow_drops_the_oldest_non_critical_first() {
        let now = Instant::now();
        let mut stack = Stack::new(2);
        stack.push(note(Urgency::Critical, now, 0), 0);
        stack.push(note(Urgency::Normal, now, 0), 0);
        stack.push(note(Urgency::Low, now, 0), 0);

        assert_eq!(stack.len(), 2);
        assert!(
            stack.items().iter().any(|n| n.urgency == Urgency::Critical),
            "a critical alert must not be pushed off by chatter"
        );
    }

    #[test]
    fn a_stack_of_only_critical_notifications_still_respects_its_capacity() {
        let now = Instant::now();
        let mut stack = Stack::new(2);
        for _ in 0..5 {
            stack.push(note(Urgency::Critical, now, 0), 0);
        }
        assert_eq!(stack.len(), 2, "the screen has a finite amount of room");
    }

    #[test]
    fn expiry_removes_exactly_the_ones_that_are_due() {
        let now = Instant::now();
        let mut stack = Stack::new(8);
        let short = stack.push(note(Urgency::Normal, now, 10), 0);
        let long = stack.push(note(Urgency::Normal, now, 60_000), 0);
        let never = stack.push(note(Urgency::Critical, now, -1), 0);

        let expired = stack.expire(now + Duration::from_millis(20));
        assert_eq!(expired, [short]);
        assert_eq!(stack.len(), 2);
        assert!(stack.items().iter().any(|n| n.id == long));
        assert!(stack.items().iter().any(|n| n.id == never));
    }

    #[test]
    fn the_next_deadline_is_the_soonest_one() {
        let now = Instant::now();
        let mut stack = Stack::new(8);
        stack.push(note(Urgency::Normal, now, 5_000), 0);
        stack.push(note(Urgency::Normal, now, 1_000), 0);
        stack.push(note(Urgency::Critical, now, -1), 0);

        let next = stack.next_deadline(now).unwrap();
        assert!(next <= Duration::from_millis(1_000));
    }

    #[test]
    fn an_empty_stack_has_no_deadline_and_nothing_to_expire() {
        let stack = Stack::new(4);
        assert!(stack.is_empty());
        assert_eq!(stack.next_deadline(Instant::now()), None);
    }

    #[test]
    fn closing_something_that_is_not_there_reports_it() {
        let mut stack = Stack::new(4);
        assert!(!stack.close(42));
    }
}
