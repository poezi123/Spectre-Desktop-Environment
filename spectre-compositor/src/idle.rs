use std::time::Instant;

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::idle_inhibit::IdleInhibitHandler;
use smithay::wayland::idle_notify::{IdleNotifierHandler, IdleNotifierState};
use smithay::{delegate_idle_inhibit, delegate_idle_notify};

use crate::state::Spectre;

impl Spectre {
    pub fn saw_activity(&mut self) {
        self.last_activity = Instant::now();
        if self.screen_is_dark {
            self.screen_is_dark = false;
            self.mark_dirty();
        }
        let seat = self.seat.clone();
        self.idle_notifier.notify_activity(&seat);
    }

    pub fn watch_for_idleness(&mut self) {
        if self.locked() || !self.idle_inhibitors.is_empty() {
            self.last_activity = Instant::now();
            return;
        }
        let idle_for = self.last_activity.elapsed();

        if let Some(after) = self.config.session.lock_after() {
            if idle_for >= after {
                self.lock_session();
            }
        }
        let Some(after) = self.config.session.blank_after() else {
            return;
        };
        if idle_for >= after && !self.screen_is_dark {
            tracing::info!("nobody is using the desktop, turning the screen dark");
            self.screen_is_dark = true;
            self.mark_dirty();
        }
    }
}

impl IdleNotifierHandler for Spectre {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_notifier
    }
}

impl IdleInhibitHandler for Spectre {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.push(surface);
        self.idle_notifier.set_is_inhibited(true);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.retain(|known| *known != surface);
        let quiet = self.idle_inhibitors.is_empty();
        self.idle_notifier.set_is_inhibited(!quiet);
    }
}

delegate_idle_notify!(Spectre);
delegate_idle_inhibit!(Spectre);
