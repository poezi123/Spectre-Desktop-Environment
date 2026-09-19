use smithay::delegate_session_lock;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::utils::{Serial, SERIAL_COUNTER};
use smithay::wayland::session_lock::{
    LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker,
};

use crate::state::Spectre;

#[derive(Debug, Default)]
pub struct Lock {
    surfaces: Vec<(WlOutput, LockSurface)>,
}

impl Lock {
    pub fn surface_for(&self, output: &Output) -> Option<&LockSurface> {
        self.surfaces
            .iter()
            .find(|(handle, _)| Output::from_resource(handle).as_ref() == Some(output))
            .map(|(_, surface)| surface)
    }

    pub fn any_surface(&self) -> Option<&LockSurface> {
        self.surfaces.first().map(|(_, surface)| surface)
    }
}

impl Spectre {
    pub fn locked(&self) -> bool {
        self.lock.is_some()
    }

    pub fn lock_session(&mut self) {
        if self.locked() {
            return;
        }
        let Some(command) = self.config.session.lock_command().map(str::to_owned) else {
            tracing::info!("no lock program is set, so the session stays unlocked");
            return;
        };
        self.launch(&command);
    }

    fn focus_the_lock(&mut self) {
        let Some(surface) = self.lock.as_ref().and_then(Lock::any_surface) else {
            return;
        };
        let wanted = surface.wl_surface().clone();
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let serial: Serial = SERIAL_COUNTER.next_serial();
        keyboard.set_focus(self, Some(wanted), serial);
    }
}

impl SessionLockHandler for Spectre {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        tracing::info!("the session is being locked");
        self.lock = Some(Lock::default());
        self.pending_lock = Some(confirmation);
        self.mark_dirty();
    }

    fn unlock(&mut self) {
        tracing::info!("the session is unlocked again");
        self.lock = None;
        self.pending_lock = None;
        let window = self.focus.clone();
        self.focus_window(window.as_ref());
        self.mark_dirty();
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let size = Output::from_resource(&output)
            .and_then(|known| known.current_mode().map(|mode| mode.size))
            .map(|size| (size.w as u32, size.h as u32))
            .unwrap_or((1920, 1080));

        surface.with_pending_state(|state| state.size = Some(size.into()));
        surface.send_configure();

        if let Some(lock) = self.lock.as_mut() {
            lock.surfaces.push((output, surface));
        }
        self.focus_the_lock();
        self.mark_dirty();
    }
}

delegate_session_lock!(Spectre);
