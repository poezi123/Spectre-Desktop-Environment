//! The compositor's end of the Spectre IPC socket.
//!
//! Clients - the panel first of all - connect, subscribe, and receive the whole
//! desktop state whenever it changes. Sending the full state rather than deltas
//! keeps both ends simple and costs a few hundred bytes per change; a panel
//! that reconnects is instantly correct instead of having to replay a log.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use smithay::desktop::Window;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use spectre_ipc::{encode_line, protocol, Desktop, Event, Request};

use crate::state::Spectre;

/// A connected client.
struct Peer {
    stream: UnixStream,
    /// Wants every state change, not just the one it asked for.
    subscribed: bool,
    /// Partial line carried over between reads.
    pending: String,
    /// Set once writing has failed; the peer is dropped on the next sweep.
    broken: bool,
}

/// Listener, connected peers and the last state we published.
pub struct Ipc {
    pub path: PathBuf,
    peers: HashMap<u64, Peer>,
    next_peer: u64,
    tokens: Vec<RegistrationToken>,
    /// Stable ids handed out to clients, so a window keeps its identity across
    /// updates even though smithay's `Window` has no id of its own.
    window_ids: Vec<(Window, protocol::WindowId)>,
    next_window_id: protocol::WindowId,
    /// The last state broadcast, to avoid sending identical updates.
    last: Option<Desktop>,
    /// Set when something changed that clients may care about.
    dirty: bool,
}

impl std::fmt::Debug for Ipc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ipc").field("path", &self.path).field("peers", &self.peers.len()).finish()
    }
}

impl Ipc {
    /// Bind the socket and start accepting clients.
    ///
    /// A stale socket from a crashed session is removed rather than treated as
    /// an error: refusing to start because the previous run died badly would be
    /// the worse failure.
    pub fn new(
        loop_handle: &LoopHandle<'static, Spectre>,
        wayland_display: &str,
    ) -> std::io::Result<Self> {
        let path = spectre_ipc::socket_path(wayland_display);
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                return Err(std::io::Error::new(
                    ErrorKind::AddrInUse,
                    format!("another compositor is already serving {}", path.display()),
                ));
            }
            let _ = std::fs::remove_file(&path);
        }

        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;

        loop_handle
            .insert_source(
                Generic::new(listener, Interest::READ, Mode::Level),
                |_, listener, state: &mut Spectre| {
                    loop {
                        match listener.accept() {
                            Ok((stream, _)) => state.accept_ipc_peer(stream),
                            Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                            Err(err) => {
                                tracing::warn!(?err, "IPC accept failed");
                                break;
                            }
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|err| std::io::Error::other(err.to_string()))?;

        Ok(Self {
            path,
            peers: HashMap::new(),
            next_peer: 0,
            tokens: Vec::new(),
            window_ids: Vec::new(),
            next_window_id: 1,
            last: None,
            dirty: true,
        })
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

}

impl Drop for Ipc {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Spectre {
    /// Register a freshly accepted client with the event loop.
    fn accept_ipc_peer(&mut self, stream: UnixStream) {
        if let Err(err) = stream.set_nonblocking(true) {
            tracing::warn!(?err, "could not configure an IPC client");
            return;
        }
        let Ok(read_half) = stream.try_clone() else {
            tracing::warn!("could not duplicate an IPC client socket");
            return;
        };

        let ipc = self.ipc.as_mut().expect("accept only runs while the IPC is up");
        let id = ipc.next_peer;
        ipc.next_peer += 1;
        ipc.peers.insert(
            id,
            Peer { stream, subscribed: false, pending: String::new(), broken: false },
        );

        let token = self.loop_handle.insert_source(
            Generic::new(read_half, Interest::READ, Mode::Level),
            move |_, stream, state: &mut Spectre| {
                // `Generic` hands back a read-only view; the peer's own
                // duplicated socket is what we actually read from.
                let _ = stream;
                state.read_ipc_peer(id);
                Ok(PostAction::Continue)
            },
        );

        match token {
            Ok(token) => {
                if let Some(ipc) = self.ipc.as_mut() {
                    ipc.tokens.push(token);
                }
                tracing::debug!(peer = id, "IPC client connected");
            }
            Err(err) => {
                tracing::warn!(?err, "could not watch an IPC client");
                if let Some(ipc) = self.ipc.as_mut() {
                    ipc.peers.remove(&id);
                }
            }
        }
    }

    /// Drain whatever a client has sent and act on complete lines.
    fn read_ipc_peer(&mut self, id: u64) {
        /// A single request longer than this is not something we understand.
        const MAX_PENDING: usize = 64 * 1024;

        let mut buf = [0u8; 4096];
        let mut requests = Vec::new();
        let mut errors = Vec::new();
        let mut closed = false;

        {
            let Some(ipc) = self.ipc.as_mut() else { return };
            let Some(peer) = ipc.peers.get_mut(&id) else { return };

            loop {
                match peer.stream.read(&mut buf) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(n) => {
                        peer.pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                        while let Some(end) = peer.pending.find('\n') {
                            let line: String = peer.pending.drain(..=end).collect();
                            match spectre_ipc::parse_line::<Request>(&line) {
                                Ok(request) => requests.push(request),
                                Err(err) => errors.push(err.to_string()),
                            }
                        }
                        if peer.pending.len() > MAX_PENDING {
                            tracing::warn!(peer = id, "dropping an oversized request");
                            peer.pending.clear();
                            peer.broken = true;
                            closed = true;
                            break;
                        }
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                    Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                    Err(err) => {
                        tracing::debug!(peer = id, ?err, "IPC read failed");
                        closed = true;
                        break;
                    }
                }
            }
        }

        for message in errors {
            tracing::debug!(peer = id, %message, "ignoring a malformed request");
            self.send_ipc(id, &Event::Error { message });
        }
        for request in requests {
            self.handle_ipc_request(id, request);
        }
        if closed {
            self.drop_ipc_peer(id);
        }
    }

    fn drop_ipc_peer(&mut self, id: u64) {
        if let Some(ipc) = self.ipc.as_mut() {
            ipc.peers.remove(&id);
        }
        tracing::debug!(peer = id, "IPC client disconnected");
    }

    fn handle_ipc_request(&mut self, id: u64, request: Request) {
        match request {
            Request::Subscribe => {
                if let Some(ipc) = self.ipc.as_mut() {
                    if let Some(peer) = ipc.peers.get_mut(&id) {
                        peer.subscribed = true;
                    }
                }
                let desktop = self.desktop_state();
                self.send_ipc(id, &Event::State(desktop));
            }
            Request::GetState => {
                let desktop = self.desktop_state();
                self.send_ipc(id, &Event::State(desktop));
            }
            Request::SwitchWorkspace { index } => {
                if self.workspaces.switch(index.saturating_sub(1) as usize) {
                    let next = self.workspaces.active().elements().last().cloned();
                    self.focus_window(next.as_ref());
                    self.mark_dirty();
                }
            }
            Request::ActivateWindow { id: window_id } => match self.window_by_id(window_id) {
                Some(window) if self.is_minimized(&window) => self.restore(&window),
                Some(window) => self.focus_window(Some(&window)),
                None => self.send_ipc(id, &unknown_window(window_id)),
            },
            Request::MinimizeWindow { id: window_id } => match self.window_by_id(window_id) {
                Some(window) => self.minimize(&window),
                None => self.send_ipc(id, &unknown_window(window_id)),
            },
            Request::CloseWindow { id: window_id } => match self.window_by_id(window_id) {
                Some(window) => {
                    if let Some(toplevel) = window.toplevel() {
                        toplevel.send_close();
                    }
                }
                None => self.send_ipc(id, &unknown_window(window_id)),
            },
            Request::SetProfile { profile } => {
                self.config.general.profile = profile;
                if let Some(effects) = profile.effects() {
                    self.config.effects = effects;
                }
                self.config.theme = profile.apply_to_theme(spectre_theme::Theme::default());
                tracing::info!(profile = profile.label(), "profile changed over IPC");
                self.mark_dirty();
            }
            Request::SetAnimations { enabled } => {
                self.config.effects.window_animations = enabled;
                self.config.theme = if enabled {
                    self.config.general.profile.apply_to_theme(spectre_theme::Theme::default())
                } else {
                    self.config.theme.clone().without_animation()
                };
                self.mark_dirty();
            }
            Request::Quit => {
                tracing::info!("session end requested over IPC");
                self.stop();
            }
        }
    }

    /// Send one event to one client, marking it broken if the write fails.
    fn send_ipc(&mut self, id: u64, event: &Event) {
        let Ok(line) = encode_line(event) else {
            return;
        };
        let Some(ipc) = self.ipc.as_mut() else { return };
        let Some(peer) = ipc.peers.get_mut(&id) else { return };

        if let Err(err) = peer.stream.write_all(line.as_bytes()) {
            // A client that stopped reading must not be allowed to block the
            // compositor, so it is dropped rather than retried.
            tracing::debug!(peer = id, ?err, "IPC write failed; dropping the client");
            peer.broken = true;
        }
    }

    /// Publish the desktop state to every subscriber, if it changed.
    pub fn publish_desktop_state(&mut self) {
        let Some(ipc) = self.ipc.as_ref() else { return };
        if !ipc.dirty || ipc.peers.is_empty() {
            return;
        }

        let desktop = self.desktop_state();
        if let Some(ipc) = self.ipc.as_mut() {
            ipc.dirty = false;
            if ipc.last.as_ref() == Some(&desktop) {
                return;
            }
            ipc.last = Some(desktop.clone());
        }

        let subscribers: Vec<u64> = self
            .ipc
            .as_ref()
            .map(|i| {
                i.peers.iter().filter(|(_, p)| p.subscribed && !p.broken).map(|(id, _)| *id).collect()
            })
            .unwrap_or_default();

        let event = Event::State(desktop);
        for id in subscribers {
            self.send_ipc(id, &event);
        }

        if let Some(ipc) = self.ipc.as_mut() {
            ipc.peers.retain(|_, peer| !peer.broken);
        }
    }

    /// Snapshot the desktop for clients.
    pub fn desktop_state(&mut self) -> Desktop {
        let active = self.workspaces.active_index();
        let mut windows = Vec::new();

        for index in 0..self.workspaces.count() {
            let Some(space) = self.workspaces.get(index) else {
                continue;
            };
            let elements: Vec<smithay::desktop::Window> = space.elements().cloned().collect();
            for window in elements {
                windows.push(self.window_info(&window, index as u8 + 1, false));
            }
        }
        let minimized: Vec<smithay::desktop::Window> =
            self.minimized.iter().map(|(w, _)| w.clone()).collect();
        for window in minimized {
            windows.push(self.window_info(&window, active as u8 + 1, true));
        }

        let workspaces = (0..self.workspaces.count())
            .map(|index| spectre_ipc::Workspace {
                index: index as u8 + 1,
                active: index == active,
                windows: windows.iter().filter(|w| w.workspace == index as u8 + 1).count() as u16,
            })
            .collect();

        Desktop {
            workspaces,
            windows,
            profile: self.config.general.profile,
            animations: self.config.effects.window_animations,
        }
    }

    fn window_info(
        &mut self,
        window: &Window,
        workspace: u8,
        minimized: bool,
    ) -> spectre_ipc::Window {
        let title = self.window_title(window);
        let app_id = self.window_app_id(window);
        spectre_ipc::Window {
            id: self.window_id(window),
            title,
            app_id,
            workspace,
            focused: self.focus.as_ref() == Some(window),
            minimized,
        }
    }

    /// The stable id for a window, assigning one on first sight.
    fn window_id(&mut self, window: &Window) -> protocol::WindowId {
        let Some(ipc) = self.ipc.as_mut() else { return 0 };
        if let Some((_, id)) = ipc.window_ids.iter().find(|(w, _)| w == window) {
            return *id;
        }
        let id = ipc.next_window_id;
        ipc.next_window_id += 1;
        ipc.window_ids.push((window.clone(), id));
        id
    }

    fn window_by_id(&self, id: protocol::WindowId) -> Option<Window> {
        let ipc = self.ipc.as_ref()?;
        ipc.window_ids.iter().find(|(_, wid)| *wid == id).map(|(w, _)| w.clone())
    }

    /// Forget ids for windows that no longer exist, so the table cannot grow
    /// without bound over a long session.
    pub fn prune_ipc_windows(&mut self) {
        let live: Vec<Window> = self
            .workspaces
            .windows()
            .cloned()
            .chain(self.minimized.iter().map(|(w, _)| w.clone()))
            .collect();
        if let Some(ipc) = self.ipc.as_mut() {
            ipc.window_ids.retain(|(w, _)| live.contains(w));
        }
    }
}

fn unknown_window(id: protocol::WindowId) -> Event {
    Event::Error { message: format!("no window with id {id}") }
}
