use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::desktop::{layer_map_for_output, Window, WindowSurfaceType};
use smithay::input::pointer::{CursorImageStatus, Focus, GrabStartData};
use smithay::output::Output;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Serial, Size};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::SurfaceCachedState;
use smithay::wayland::seat::WaylandFocus;
use spectre_config::Direction;

use crate::animation::{Closing, LauncherClosing, Pop, Slide};
use crate::grabs::{resize_icon, ActiveResize, ResizeGrab, BTN_LEFT};
use crate::window_menu::{MenuFor, MenuItem, WindowMenu};
use crate::render::{decorations, Edges, Frame, Part, LAUNCHER_NAMESPACE};
use crate::state::Spectre;

const CASCADE_STEP: i32 = 28;
const MOVE_STEP: i32 = 64;

impl Spectre {
    pub fn active_output(&self) -> Option<Output> {
        let pos = self.pointer_position();
        self.workspaces
            .active()
            .output_under(pos)
            .next()
            .cloned()
            .or_else(|| self.outputs().first().cloned())
    }

    pub fn working_area(&self, output: &Output) -> Rectangle<i32, Logical> {
        let geometry = self
            .workspaces
            .output_geometry(output)
            .unwrap_or_else(|| Rectangle::from_size(Size::from((0, 0))));
        let mut area = layer_map_for_output(output).non_exclusive_zone();
        area.loc += geometry.loc;
        if area.size.w <= 0 || area.size.h <= 0 {
            return geometry;
        }
        area
    }

    fn top_inset(&self, window: &Window) -> i32 {
        let m = self.config.theme.metrics;
        if self.is_decorated(window) {
            m.titlebar_height as i32 + m.border_width as i32
        } else {
            0
        }
    }

    pub fn place_window(&mut self, window: Window) {
        let Some(output) = self.active_output() else {
            tracing::warn!("no output to place a window on; dropping it");
            return;
        };

        let area = self.working_area(&output);
        let size = window.geometry().size;
        let top = self.top_inset(&window);
        let border = if self.is_decorated(&window) {
            self.config.theme.metrics.border_width as i32
        } else {
            0
        };

        let mut loc = Point::from((
            area.loc.x + (area.size.w - size.w).max(0) / 2,
            area.loc.y + top + (area.size.h - size.h - top - border).max(0) / 2,
        ));

        let occupied = |space: &smithay::desktop::Space<Window>, p: Point<i32, Logical>| {
            space.elements().any(|w| space.element_location(w) == Some(p))
        };
        let mut guard = 0;
        while occupied(self.workspaces.active(), loc) && guard < 16 {
            loc += Point::from((CASCADE_STEP, CASCADE_STEP));
            guard += 1;
        }
        loc.x = loc.x.min((area.loc.x + area.size.w - size.w).max(area.loc.x));
        loc.y = loc.y.min((area.loc.y + area.size.h - size.h).max(area.loc.y + top));
        loc.y = loc.y.max(area.loc.y + top);

        self.workspaces.active_mut().map_element(window.clone(), loc, true);
        self.tell_x11_where(&window);
        self.start_opening(&window);
        self.focus_window(Some(&window));
        self.mark_dirty();
    }

    pub fn close_window(&self, window: &Window) {
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_close();
            return;
        }
        if let Some(x11) = window.x11_surface() {
            let _ = x11.close();
        }
    }

    fn tell_x11_where(&self, window: &Window) {
        let Some(x11) = window.x11_surface() else {
            return;
        };
        if x11.is_override_redirect() {
            return;
        }
        let Some(location) = self.workspaces.active().element_location(window) else {
            return;
        };
        let size = window.geometry().size;
        let _ = x11.configure(Rectangle::new(location, size));
    }

    fn place_x11_centered(&mut self, window: &Window) {
        let Some(x11) = window.x11_surface() else {
            return;
        };
        let Some(output) = self.active_output() else {
            return;
        };
        let area = self.working_area(&output);
        let top = self.top_inset(window);
        let width = (area.size.w * 2 / 3).max(1);
        let height = ((area.size.h - top) * 2 / 3).max(1);
        let x = area.loc.x + (area.size.w - width) / 2;
        let y = area.loc.y + top + (area.size.h - top - height) / 2;
        let location = Point::from((x, y));
        self.workspaces.active_mut().map_element(window.clone(), location, true);
        let _ = x11.configure(Rectangle::new(location, Size::from((width, height))));
    }

    fn start_opening(&mut self, window: &Window) {
        if !self.config.effects.window_animations {
            return;
        }
        let pop = Pop::opening(std::time::Instant::now(), self.config.effects.animation_speed);
        self.opening.retain(|(opening, _)| opening != window);
        self.opening.push((window.clone(), pop));
    }

    pub fn opening_pop(&self, window: &Window) -> Option<Pop> {
        for (opening, pop) in &self.opening {
            if opening == window {
                return Some(*pop);
            }
        }
        None
    }

    fn start_closing(&mut self, window: &Window) {
        if !self.config.effects.window_animations {
            return;
        }
        if let Some(x11) = window.x11_surface() {
            if x11.is_override_redirect() {
                return;
            }
        }
        let metrics = self.config.theme.metrics;
        let decorated = self.is_decorated(window);
        for workspace in 0..self.workspaces.count() {
            let Some(space) = self.workspaces.get(workspace) else {
                continue;
            };
            let Some(geometry) = space.element_geometry(window) else {
                continue;
            };
            let outer = Frame::new(geometry, &metrics, decorated).outer;
            let pop = Pop::closing(std::time::Instant::now(), self.config.effects.animation_speed);
            let key = crate::render::element_key(window);
            self.closing.push(Closing { key, workspace, outer, pop });
            return;
        }
    }

    pub fn finish_window_animations(&mut self) {
        let now = std::time::Instant::now();
        let before = self.opening.len() + self.closing.len();
        self.opening.retain(|(_, pop)| !pop.is_done(now));
        self.closing.retain(|closing| !closing.pop.is_done(now));
        let mut changed = self.opening.len() + self.closing.len() != before;
        if let Some(slide) = self.launcher_opening {
            if slide.is_done(now) {
                self.launcher_opening = None;
                changed = true;
            }
        }
        if let Some(closing) = self.launcher_closing.as_ref() {
            if closing.slide.is_done(now) {
                self.launcher_closing = None;
                changed = true;
            }
        }
        if changed {
            self.mark_dirty();
        }
    }

    pub fn start_launcher_slide(&mut self, surface: &WlSurface) {
        if self.launcher_shown || !self.is_launcher(surface) {
            return;
        }
        let has_buffer = with_renderer_surface_state(surface, |state| state.buffer().is_some());
        if has_buffer != Some(true) {
            return;
        }
        self.launcher_shown = true;
        self.launcher_closing = None;
        if self.config.effects.window_animations {
            let slide = Slide::opening(std::time::Instant::now(), self.config.effects.animation_speed);
            self.launcher_opening = Some(slide);
        }
        self.mark_dirty();
    }

    fn is_launcher(&self, surface: &WlSurface) -> bool {
        for output in self.outputs() {
            let map = layer_map_for_output(&output);
            if let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) {
                return layer.namespace() == LAUNCHER_NAMESPACE;
            }
        }
        false
    }

    pub fn start_launcher_close(&mut self, output: Output) {
        self.launcher_shown = false;
        self.launcher_opening = None;
        if !self.config.effects.window_animations {
            return;
        }
        let slide = Slide::closing(std::time::Instant::now(), self.config.effects.animation_speed);
        self.launcher_closing = Some(LauncherClosing { output, slide });
        self.mark_dirty();
    }

    pub fn unmap_window(&mut self, window: &Window) {
        self.start_closing(window);
        self.opening.retain(|(opening, _)| opening != window);
        self.snapped.retain(|(snapped, _, _)| snapped != window);
        self.desktop_hidden.retain(|hidden| hidden != window);
        let menu_window = match self.window_menu.as_ref().map(|menu| &menu.target) {
            Some(MenuFor::Window(open)) => Some(open.clone()),
            _ => None,
        };
        if menu_window.as_ref() == Some(window) {
            self.window_menu = None;
        }
        for space in self.workspaces.iter_mut() {
            space.unmap_elem(window);
        }
        if self.focus.as_ref() == Some(window) {
            self.focus = None;
            let next = self.workspaces.active().elements().last().cloned();
            self.focus_window(next.as_ref());
        }
        self.mark_dirty();
    }

    pub fn focus_window(&mut self, window: Option<&Window>) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();

        let windows: Vec<Window> = self.workspaces.active().elements().cloned().collect();
        for w in &windows {
            let active = Some(w) == window;
            if let Some(toplevel) = w.toplevel() {
                toplevel.with_pending_state(|state| {
                    if active {
                        state.states.set(xdg_toplevel::State::Activated);
                    } else {
                        state.states.unset(xdg_toplevel::State::Activated);
                    }
                });
                toplevel.send_pending_configure();
            }
            if let Some(x11) = w.x11_surface() {
                if !x11.is_override_redirect() {
                    let _ = x11.set_activated(active);
                }
            }
        }

        match window {
            Some(w) => {
                self.workspaces.active_mut().raise_element(w, true);
                if let Some(x11) = w.x11_surface() {
                    if let Some(xwm) = self.xwm.as_mut() {
                        let _ = xwm.raise_window(x11);
                    }
                }
                keyboard.set_focus(self, w.wl_surface().map(|s| s.into_owned()), serial);
                self.focus = Some(w.clone());
            }
            None => {
                keyboard.set_focus(self, None, serial);
                self.focus = None;
            }
        }
        self.mark_dirty();
    }

    pub fn cycle_focus(&mut self, forward: bool) {
        let mut windows: Vec<Window> = self.workspaces.active().elements().cloned().collect();
        windows.extend(self.minimized.iter().map(|(w, _)| w.clone()));
        if windows.is_empty() {
            return;
        }
        let current = self
            .focus
            .as_ref()
            .and_then(|f| windows.iter().position(|w| w == f))
            .unwrap_or(0);
        let n = windows.len();
        let next = if forward { (current + 1) % n } else { (current + n - 1) % n };
        let target = windows[next].clone();
        if self.is_minimized(&target) {
            self.restore(&target);
        } else {
            self.focus_window(Some(&target));
        }
    }

    pub fn focus_direction(&mut self, direction: Direction) {
        let space = self.workspaces.active();
        let Some(current) = self.focus.clone() else {
            let first = space.elements().last().cloned();
            self.focus_window(first.as_ref());
            return;
        };
        let Some(origin) = space.element_geometry(&current).map(center) else {
            return;
        };

        let best = space
            .elements()
            .filter(|w| **w != current)
            .filter_map(|w| space.element_geometry(w).map(|g| (w.clone(), center(g))))
            .filter(|(_, c)| in_direction(origin, *c, direction))
            .min_by_key(|(_, c)| distance_sq(origin, *c))
            .map(|(w, _)| w);

        if let Some(window) = best {
            self.focus_window(Some(&window));
        }
    }

    pub fn move_window_to(&mut self, window: &Window, location: Point<i32, Logical>) {
        let Some(output) = self.active_output() else {
            return;
        };
        let area = self.working_area(&output);
        let top = self.top_inset(window);
        let size = window.geometry().size;

        let mut loc = location;
        const MIN_VISIBLE: i32 = 48;
        loc.x = loc.x.clamp(
            area.loc.x - (size.w - MIN_VISIBLE).max(0),
            area.loc.x + area.size.w - MIN_VISIBLE,
        );
        loc.y = loc.y.clamp(area.loc.y + top, area.loc.y + area.size.h - MIN_VISIBLE);

        self.workspaces.active_mut().map_element(window.clone(), loc, false);
        self.tell_x11_where(window);
        self.mark_dirty();
    }

    pub fn minimize(&mut self, window: &Window) {
        let Some(location) = self.workspaces.active().element_location(window) else {
            return;
        };
        if self.minimized.iter().any(|(w, _)| w == window) {
            return;
        }
        self.workspaces.active_mut().unmap_elem(window);
        self.minimized.push((window.clone(), location));

        if self.focus.as_ref() == Some(window) {
            let next = self.workspaces.active().elements().last().cloned();
            self.focus_window(next.as_ref());
        }
        self.mark_dirty();
    }

    pub fn restore(&mut self, window: &Window) {
        let Some(index) = self.minimized.iter().position(|(w, _)| w == window) else {
            return;
        };
        let (window, location) = self.minimized.remove(index);
        self.workspaces.active_mut().map_element(window.clone(), location, true);
        self.tell_x11_where(&window);
        self.focus_window(Some(&window));
    }

    pub fn is_minimized(&self, window: &Window) -> bool {
        self.minimized.iter().any(|(w, _)| w == window)
    }

    pub fn snap_focused(&mut self, side: Direction) {
        let Some(window) = self.focus.clone() else {
            return;
        };
        let Some(output) = self.active_output() else {
            return;
        };
        let Some(location) = self.workspaces.active().element_location(&window) else {
            return;
        };

        let mut restore = Rectangle::new(location, window.geometry().size);
        let mut already_there = false;
        for (snapped, snapped_side, before) in &self.snapped {
            if snapped == &window {
                restore = *before;
                already_there = *snapped_side == side;
            }
        }
        self.snapped.retain(|(snapped, _, _)| snapped != &window);

        if already_there {
            self.place_exactly(&window, restore);
            return;
        }

        if self.has_state(&window, xdg_toplevel::State::Maximized) {
            self.set_maximized(&window, false);
        }

        let area = self.working_area(&output);
        let top = self.top_inset(&window);
        let mut border = 0;
        if self.is_decorated(&window) {
            border = self.config.theme.metrics.border_width as i32;
        }
        let half = area.size.w / 2;
        let mut left = area.loc.x;
        if side == Direction::Right {
            left = area.loc.x + half;
        }
        let snapped_location = Point::from((left + border, area.loc.y + top));
        let snapped_size = Size::from(((half - border * 2).max(1), (area.size.h - top - border).max(1)));
        self.place_exactly(&window, Rectangle::new(snapped_location, snapped_size));
        self.snapped.push((window, side, restore));
    }

    fn place_exactly(&mut self, window: &Window, rect: Rectangle<i32, Logical>) {
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Maximized);
                state.size = Some(rect.size);
            });
            toplevel.send_pending_configure();
        }
        self.workspaces.active_mut().map_element(window.clone(), rect.loc, true);
        if let Some(x11) = window.x11_surface() {
            let _ = x11.configure(rect);
        }
        self.mark_dirty();
    }

    pub fn toggle_show_desktop(&mut self) {
        let windows: Vec<Window> = self.workspaces.active().elements().cloned().collect();
        if windows.is_empty() {
            let hidden = std::mem::take(&mut self.desktop_hidden);
            for window in hidden {
                if self.is_minimized(&window) {
                    self.restore(&window);
                }
            }
            return;
        }
        self.desktop_hidden.clear();
        for window in windows {
            self.minimize(&window);
            self.desktop_hidden.push(window);
        }
    }

    pub fn edges_under_pointer(&self) -> Option<(Window, Edges)> {
        let pointer = self.pointer_position();
        let metrics = self.config.theme.metrics;
        let space = self.workspaces.active();

        for window in space.elements().rev() {
            let Some(geometry) = space.element_geometry(window) else {
                continue;
            };
            let decorated = self.is_decorated(window);
            let frame = Frame::new(geometry, &metrics, decorated);
            let resizable = decorated
                && !self.has_state(window, xdg_toplevel::State::Maximized)
                && !self.has_state(window, xdg_toplevel::State::Fullscreen);
            if resizable {
                if let Some(edges) = decorations::edges_at(&frame, pointer) {
                    return Some((window.clone(), edges));
                }
            }
            if frame.outer.to_f64().contains(pointer) {
                return None;
            }
        }
        None
    }

    pub fn is_resizing(&self) -> bool {
        match self.resize.as_ref() {
            Some(resize) => !resize.released,
            None => false,
        }
    }

    pub fn start_resize(&mut self, window: &Window, edges: Edges, serial: Serial) {
        if edges.is_empty() {
            return;
        }
        let Some(location) = self.workspaces.active().element_location(window) else {
            return;
        };
        let initial = Rectangle::new(location, window.geometry().size);
        self.snapped.retain(|(snapped, _, _)| snapped != window);
        self.resize = Some(ActiveResize { window: window.clone(), edges, initial, released: false });
        self.cursor_status = CursorImageStatus::Named(resize_icon(edges));
        self.edge_cursor = true;

        let start_data = GrabStartData {
            focus: None,
            button: BTN_LEFT,
            location: self.pointer_position(),
        };
        let grab = ResizeGrab::new(start_data, window.clone(), edges, initial);
        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    pub fn resize_window_to(&mut self, window: &Window, size: Size<i32, Logical>) {
        let Some(resize) = self.resize.clone() else {
            return;
        };
        if &resize.window != window {
            return;
        }
        let size = self.respect_minimum_size(window, size);
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Resizing);
                state.size = Some(size);
            });
            toplevel.send_pending_configure();
        }
        if let Some(x11) = window.x11_surface() {
            let location = resize.location_for(size);
            self.workspaces.active_mut().map_element(window.clone(), location, false);
            let _ = x11.configure(Rectangle::new(location, size));
        }
        self.mark_dirty();
    }

    fn respect_minimum_size(&self, window: &Window, size: Size<i32, Logical>) -> Size<i32, Logical> {
        let Some(toplevel) = window.toplevel() else {
            return size;
        };
        let minimum = with_states(toplevel.wl_surface(), |states| {
            states.cached_state.get::<SurfaceCachedState>().current().min_size
        });
        Size::from((size.w.max(minimum.w), size.h.max(minimum.h)))
    }

    pub fn follow_resize(&mut self, window: &Window) {
        let Some(resize) = self.resize.clone() else {
            return;
        };
        if &resize.window != window {
            return;
        }
        let location = resize.location_for(window.geometry().size);
        self.workspaces.active_mut().map_element(window.clone(), location, false);
        if resize.released {
            self.resize = None;
        }
        self.mark_dirty();
    }

    pub fn open_window_menu(&mut self, window: &Window, at: Point<i32, Logical>) {
        let Some(output) = self.active_output() else {
            return;
        };
        let Some(area) = self.workspaces.output_geometry(&output) else {
            return;
        };
        let maximized = self.has_state(window, xdg_toplevel::State::Maximized);
        self.window_menu = Some(WindowMenu::for_window(window.clone(), at, maximized, area));
        self.cursor_status = CursorImageStatus::default_named();
        self.edge_cursor = false;
        self.mark_dirty();
    }

    pub fn open_desktop_menu(&mut self, at: Point<i32, Logical>) {
        let Some(output) = self.active_output() else {
            return;
        };
        let Some(area) = self.workspaces.output_geometry(&output) else {
            return;
        };
        self.window_menu = Some(WindowMenu::for_desktop(at, area));
        self.cursor_status = CursorImageStatus::default_named();
        self.edge_cursor = false;
        self.mark_dirty();
    }

    pub fn close_window_menu(&mut self) {
        self.window_menu = None;
        self.mark_dirty();
    }

    pub fn menu_pointer_moved(&mut self) {
        let pointer = self.pointer_position();
        let Some(menu) = self.window_menu.as_mut() else {
            return;
        };
        let hovered = menu.item_at(pointer);
        if menu.hovered != hovered {
            menu.hovered = hovered;
            self.mark_dirty();
        }
    }

    pub fn menu_button(&mut self, pressed: bool) {
        if !pressed {
            return;
        }
        let pointer = self.pointer_position();
        let Some(menu) = self.window_menu.as_ref() else {
            return;
        };
        match menu.item_at(pointer) {
            Some(index) => self.activate_menu_item(index),
            None => self.close_window_menu(),
        }
    }

    pub fn activate_menu_item(&mut self, index: usize) {
        let Some(menu) = self.window_menu.take() else {
            return;
        };
        self.mark_dirty();
        let Some(item) = menu.items.get(index).copied() else {
            return;
        };
        let window = match menu.target {
            MenuFor::Window(window) => window,
            MenuFor::Desktop => {
                self.activate_desktop_item(item);
                return;
            }
        };
        match item {
            MenuItem::Minimize => self.minimize(&window),
            MenuItem::Maximize => self.set_maximized(&window, true),
            MenuItem::Restore => self.set_maximized(&window, false),
            MenuItem::Fullscreen => {
                let on = self.has_state(&window, xdg_toplevel::State::Fullscreen);
                self.set_fullscreen(&window, !on);
            }
            MenuItem::SnapLeft => {
                self.focus_window(Some(&window));
                self.snap_focused(Direction::Left);
            }
            MenuItem::SnapRight => {
                self.focus_window(Some(&window));
                self.snap_focused(Direction::Right);
            }
            MenuItem::PreviousWorkspace => {
                self.focus_window(Some(&window));
                self.run_action(spectre_config::Action::MoveToPrevWorkspace);
            }
            MenuItem::NextWorkspace => {
                self.focus_window(Some(&window));
                self.run_action(spectre_config::Action::MoveToNextWorkspace);
            }
            MenuItem::Close => self.close_window(&window),
            MenuItem::Terminal
            | MenuItem::Settings
            | MenuItem::Wallpaper
            | MenuItem::ShowDesktop
            | MenuItem::Overview => self.activate_desktop_item(item),
        }
    }

    fn activate_desktop_item(&mut self, item: MenuItem) {
        match item {
            MenuItem::Terminal => {
                let command = self.config.terminal.command().to_owned();
                self.launch(&command);
            }
            MenuItem::Settings | MenuItem::Wallpaper => self.launch("spectre-settings"),
            MenuItem::ShowDesktop => self.toggle_show_desktop(),
            MenuItem::Overview => self.open_overview(),
            _ => {}
        }
    }

    pub fn finish_resize(&mut self, window: &Window) {
        match window.toplevel() {
            Some(toplevel) => {
                toplevel.with_pending_state(|state| {
                    state.states.unset(xdg_toplevel::State::Resizing);
                });
                toplevel.send_pending_configure();
                if let Some(resize) = self.resize.as_mut() {
                    resize.released = true;
                }
            }
            None => self.resize = None,
        }
        self.cursor_status = CursorImageStatus::default_named();
        self.edge_cursor = false;
        self.mark_dirty();
    }

    pub fn move_direction(&mut self, direction: Direction) {
        let Some(window) = self.focus.clone() else {
            return;
        };
        let Some(output) = self.active_output() else {
            return;
        };
        let area = self.working_area(&output);
        let space = self.workspaces.active_mut();
        let Some(mut loc) = space.element_location(&window) else {
            return;
        };
        let size = window.geometry().size;

        match direction {
            Direction::Left => loc.x -= MOVE_STEP,
            Direction::Right => loc.x += MOVE_STEP,
            Direction::Up => loc.y -= MOVE_STEP,
            Direction::Down => loc.y += MOVE_STEP,
        }
        loc.x = loc.x.clamp(area.loc.x, (area.loc.x + area.size.w - size.w).max(area.loc.x));
        loc.y = loc.y.clamp(area.loc.y, (area.loc.y + area.size.h - size.h).max(area.loc.y));

        space.map_element(window.clone(), loc, true);
        self.tell_x11_where(&window);
        self.mark_dirty();
    }

    pub fn set_maximized(&mut self, window: &Window, maximized: bool) {
        let Some(output) = self.active_output() else {
            return;
        };
        let area = self.working_area(&output);
        let top = self.top_inset(window);
        let border = if self.is_decorated(window) {
            self.config.theme.metrics.border_width as i32
        } else {
            0
        };
        let size = Size::from((
            (area.size.w - border * 2).max(1),
            (area.size.h - top - border).max(1),
        ));
        let location = Point::from((area.loc.x + border, area.loc.y + top));

        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                if maximized {
                    state.states.set(xdg_toplevel::State::Maximized);
                    state.size = Some(size);
                } else {
                    state.states.unset(xdg_toplevel::State::Maximized);
                    state.size = None;
                }
            });
            toplevel.send_pending_configure();
            if maximized {
                self.workspaces.active_mut().map_element(window.clone(), location, true);
            }
        } else if let Some(x11) = window.x11_surface() {
            let _ = x11.set_maximized(maximized);
            if maximized {
                self.workspaces.active_mut().map_element(window.clone(), location, true);
                let _ = x11.configure(Rectangle::new(location, size));
            } else {
                self.place_x11_centered(window);
            }
        }
        self.mark_dirty();
    }

    pub fn set_fullscreen(&mut self, window: &Window, fullscreen: bool) {
        let Some(output) = self.active_output() else {
            return;
        };
        let Some(geometry) = self.workspaces.output_geometry(&output) else {
            return;
        };

        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|state| {
                if fullscreen {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                    state.size = Some(geometry.size);
                } else {
                    state.states.unset(xdg_toplevel::State::Fullscreen);
                    state.size = None;
                }
            });
            toplevel.send_pending_configure();
            if fullscreen {
                self.workspaces.active_mut().map_element(window.clone(), geometry.loc, true);
            }
        } else if let Some(x11) = window.x11_surface() {
            let _ = x11.set_fullscreen(fullscreen);
            if fullscreen {
                self.workspaces.active_mut().map_element(window.clone(), geometry.loc, true);
                let _ = x11.configure(geometry);
            } else {
                self.place_x11_centered(window);
            }
        }
        self.mark_dirty();
    }

    pub fn focused_has_state(&self, wanted: xdg_toplevel::State) -> bool {
        self.focus.as_ref().map(|w| self.has_state(w, wanted)).unwrap_or(false)
    }

    pub fn close_focused(&mut self) {
        if let Some(window) = self.focus.clone() {
            self.close_window(&window);
        }
    }

    pub fn switch_workspace(&mut self, index: usize) -> bool {
        let from = self.workspaces.active_index();
        if !self.workspaces.switch(index) {
            return false;
        }
        self.begin_transition(from);
        true
    }

    pub fn open_overview(&mut self) {
        if self.overview.is_some() || self.workspaces.count() < 2 {
            return;
        }
        self.transition = None;
        let faces = self.workspaces.count();
        let active = self.workspaces.active_index();
        self.overview = Some(crate::overview::Overview::open(faces, active));
        self.mark_dirty();
    }

    pub fn close_overview(&mut self, choice: Option<usize>) {
        if self.overview.take().is_none() {
            return;
        }
        if let Some(index) = choice {
            if self.workspaces.switch(index) {
                let next = self.workspaces.active().elements().last().cloned();
                self.focus_window(next.as_ref());
            }
        }
        self.mark_dirty();
    }

    pub fn switch_workspace_relative(&mut self, delta: isize) -> bool {
        let from = self.workspaces.active_index();
        if !self.workspaces.switch_relative(delta) {
            return false;
        }
        self.begin_transition(from);
        true
    }

    fn begin_transition(&mut self, from: usize) {
        let effects = &self.config.effects;
        self.transition = crate::transition::Transition::start(
            from,
            self.workspaces.active_index(),
            effects.workspace_transition,
            effects.transition_duration_ms(),
            std::time::Instant::now(),
        );

        let next = self.workspaces.active().elements().last().cloned();
        self.focus_window(next.as_ref());
        self.mark_dirty();
    }

    pub fn finish_transition(&mut self) -> bool {
        let now = std::time::Instant::now();
        if self.transition.as_ref().is_some_and(|t| t.is_done(now)) {
            self.transition = None;
            self.mark_dirty();
            return true;
        }
        false
    }

    pub fn update_layer_focus(&mut self) {
        let target = self.outputs().into_iter().find_map(|output| {
            let map = layer_map_for_output(&output);
            let surface = map
                .layers()
                .rev()
                .find(|layer| layer.can_receive_keyboard_focus())
                .map(|layer| layer.wl_surface().clone());
            surface
        });

        if target == self.layer_focus {
            return;
        }
        self.layer_focus = target.clone();

        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();

        match target {
            Some(surface) => keyboard.set_focus(self, Some(surface), serial),
            None => {
                let window = self.focus.clone();
                self.focus_window(window.as_ref());
            }
        }
        self.mark_dirty();
    }

    pub fn reflow_output(&mut self, output: &Output) {
        layer_map_for_output(output).arrange();

        let windows: Vec<Window> = self.workspaces.active().elements().cloned().collect();
        for window in windows {
            if self.is_maximized(&window) {
                self.set_maximized(&window, true);
            }
            if self.is_fullscreen(&window) {
                self.set_fullscreen(&window, true);
            }
        }
        self.bring_windows_back(output);
        self.mark_dirty();
    }

    fn bring_windows_back(&mut self, output: &Output) {
        let area = self.working_area(output);
        let metrics = self.config.theme.metrics;
        for index in 0..self.workspaces.count() {
            let Some(space) = self.workspaces.get(index) else {
                continue;
            };
            let windows: Vec<Window> = space.elements().cloned().collect();
            for window in windows {
                if self.is_maximized(&window) || self.is_fullscreen(&window) {
                    continue;
                }
                let Some(space) = self.workspaces.get(index) else {
                    continue;
                };
                let Some(geometry) = space.element_geometry(&window) else {
                    continue;
                };
                let frame = Frame::new(geometry, &metrics, self.is_decorated(&window));
                let wanted = back_on_screen(frame.outer, area);
                if wanted == frame.outer.loc {
                    continue;
                }
                let moved = geometry.loc + (wanted - frame.outer.loc);
                if let Some(space) = self.workspaces.get_mut(index) {
                    space.map_element(window.clone(), moved, false);
                }
            }
        }
    }

    pub fn decoration_under_pointer(&self) -> Option<(Window, Part)> {
        let pointer = self.pointer_position();
        let metrics = self.config.theme.metrics;
        let space = self.workspaces.active();

        for window in space.elements().rev() {
            let Some(geometry) = space.element_geometry(window) else {
                continue;
            };
            let frame = Frame::new(geometry, &metrics, self.is_decorated(window));
            if let Some(part) = decorations::part_at(&frame, &metrics, pointer) {
                return Some((window.clone(), part));
            }
            if frame.window.contains(Point::<i32, Logical>::from((
                pointer.x.floor() as i32,
                pointer.y.floor() as i32,
            ))) {
                return None;
            }
        }
        None
    }

    pub fn window_under_pointer(&self) -> Option<Window> {
        let pos = self.pointer_position();
        self.workspaces
            .active()
            .element_under(pos)
            .map(|(w, _)| w.clone())
    }

    pub fn surface_under_pointer(
        &self,
    ) -> Option<(
        smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        Point<f64, Logical>,
    )> {
        let pos = self.pointer_position();
        let output = self.active_output()?;
        if let Some(lock) = self.lock.as_ref() {
            let surface = lock.surface_for(&output)?;
            return Some((surface.wl_surface().clone(), Point::from((0.0, 0.0))));
        }
        let output_geo = self.workspaces.output_geometry(&output)?;
        let layers = layer_map_for_output(&output);

        let covered = self.a_window_covers_the_output();
        let above = layers
            .layer_under(smithay::wayland::shell::wlr_layer::Layer::Overlay, pos)
            .or_else(|| {
                if covered {
                    return None;
                }
                layers.layer_under(smithay::wayland::shell::wlr_layer::Layer::Top, pos)
            });

        if let Some(layer) = above {
            let loc = layers.layer_geometry(layer)?.loc + output_geo.loc;
            return layer
                .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                .map(|(s, p)| (s, (p + loc).to_f64()));
        }

        let space = self.workspaces.active();
        let metrics = self.config.theme.metrics;
        for window in space.elements().rev() {
            let Some(geometry) = space.element_geometry(window) else {
                continue;
            };
            let Some(location) = space.element_location(window) else {
                continue;
            };
            let render_location = location - window.geometry().loc;
            let local = pos - render_location.to_f64();
            if let Some((surface, point)) = window.surface_under(local, WindowSurfaceType::ALL) {
                return Some((surface, (point + render_location).to_f64()));
            }
            let decorated = self.is_decorated(window);
            let frame = Frame::new(geometry, &metrics, decorated);
            if frame.outer.to_f64().contains(pos) {
                return None;
            }
            if decorated && decorations::edges_at(&frame, pos).is_some() {
                return None;
            }
        }

        let below = layers
            .layer_under(smithay::wayland::shell::wlr_layer::Layer::Bottom, pos)
            .or_else(|| {
                layers.layer_under(smithay::wayland::shell::wlr_layer::Layer::Background, pos)
            })?;
        let loc = layers.layer_geometry(below)?.loc + output_geo.loc;
        below
            .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
            .map(|(s, p)| (s, (p + loc).to_f64()))
    }
}

fn center(rect: Rectangle<i32, Logical>) -> Point<i32, Logical> {
    Point::from((rect.loc.x + rect.size.w / 2, rect.loc.y + rect.size.h / 2))
}

fn distance_sq(a: Point<i32, Logical>, b: Point<i32, Logical>) -> i64 {
    let dx = (a.x - b.x) as i64;
    let dy = (a.y - b.y) as i64;
    dx * dx + dy * dy
}

fn in_direction(
    origin: Point<i32, Logical>,
    target: Point<i32, Logical>,
    direction: Direction,
) -> bool {
    let dx = target.x - origin.x;
    let dy = target.y - origin.y;
    match direction {
        Direction::Left => dx < 0 && dx.abs() >= dy.abs(),
        Direction::Right => dx > 0 && dx.abs() >= dy.abs(),
        Direction::Up => dy < 0 && dy.abs() > dx.abs(),
        Direction::Down => dy > 0 && dy.abs() > dx.abs(),
    }
}

pub const STAYS_VISIBLE: i32 = 80;

pub fn back_on_screen(
    window: Rectangle<i32, Logical>,
    area: Rectangle<i32, Logical>,
) -> Point<i32, Logical> {
    if area.size.w <= 0 || area.size.h <= 0 {
        return window.loc;
    }
    let room = STAYS_VISIBLE.min(window.size.w).min(window.size.h).max(1);
    let fits_across = window.size.w <= area.size.w;
    let fits_down = window.size.h <= area.size.h;

    let left = match fits_across {
        true => area.loc.x,
        false => area.loc.x - (window.size.w - room),
    };
    let right = match fits_across {
        true => area.loc.x + area.size.w - window.size.w,
        false => area.loc.x + area.size.w - room,
    };
    let top = area.loc.y;
    let bottom = match fits_down {
        true => area.loc.y + area.size.h - window.size.h,
        false => area.loc.y + area.size.h - room,
    };
    Point::from((window.loc.x.clamp(left, right.max(left)), window.loc.y.clamp(top, bottom.max(top))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((0, 0)), Size::from((1920, 1040)))
    }

    #[test]
    fn a_window_inside_the_screen_is_left_alone() {
        let window = Rectangle::new(Point::from((100, 100)), Size::from((800, 600)));
        assert_eq!(back_on_screen(window, area()), window.loc);
    }

    #[test]
    fn a_window_that_fits_ends_up_fully_on_the_screen() {
        let window = Rectangle::new(Point::from((3000, 200)), Size::from((800, 600)));
        let moved = back_on_screen(window, area());
        assert_eq!(moved.y, 200, "it only moves as far as it has to");
        assert_eq!(moved.x, 1920 - 800, "it comes to rest against the right edge");
    }

    #[test]
    fn a_window_wider_than_the_screen_keeps_a_handle_on_it() {
        let window = Rectangle::new(Point::from((3000, 200)), Size::from((2400, 600)));
        let moved = back_on_screen(window, area());
        assert!(moved.x + STAYS_VISIBLE <= 1920, "a piece of it must be reachable");
        assert!(moved.x < 3000);
    }

    #[test]
    fn a_window_below_the_screen_comes_back_up() {
        let window = Rectangle::new(Point::from((100, 2000)), Size::from((800, 600)));
        let moved = back_on_screen(window, area());
        assert_eq!(moved.y, 1040 - 600);
    }

    #[test]
    fn a_title_bar_never_ends_up_above_the_screen() {
        let window = Rectangle::new(Point::from((100, -400)), Size::from((800, 600)));
        assert_eq!(back_on_screen(window, area()).y, 0);
    }

    #[test]
    fn a_window_may_hang_over_the_left_edge_only_if_it_is_too_big() {
        let fits = Rectangle::new(Point::from((-5000, 100)), Size::from((800, 600)));
        assert_eq!(back_on_screen(fits, area()).x, 0);

        let huge = Rectangle::new(Point::from((-5000, 100)), Size::from((2400, 600)));
        assert_eq!(back_on_screen(huge, area()).x, -(2400 - STAYS_VISIBLE));
    }

    #[test]
    fn a_screen_with_no_size_moves_nothing() {
        let nothing = Rectangle::new(Point::from((0, 0)), Size::from((0, 0)));
        let window = Rectangle::new(Point::from((10, 10)), Size::from((100, 100)));
        assert_eq!(back_on_screen(window, nothing), window.loc);
    }

    fn p(x: i32, y: i32) -> Point<i32, Logical> {
        Point::from((x, y))
    }

    #[test]
    fn direction_test_picks_the_dominant_axis() {
        let origin = p(100, 100);
        assert!(in_direction(origin, p(500, 120), Direction::Right));
        assert!(!in_direction(origin, p(500, 120), Direction::Up));
        assert!(in_direction(origin, p(110, 10), Direction::Up));
        assert!(!in_direction(origin, p(110, 10), Direction::Right));
    }

    #[test]
    fn nothing_is_in_a_direction_from_itself() {
        let origin = p(50, 50);
        for d in [Direction::Left, Direction::Right, Direction::Up, Direction::Down] {
            assert!(!in_direction(origin, origin, d));
        }
    }

    #[test]
    fn centre_of_a_rectangle_is_its_middle() {
        let r = Rectangle::new(p(10, 20), Size::from((100, 50)));
        assert_eq!(center(r), p(60, 45));
    }

    #[test]
    fn distance_is_symmetric_and_zero_at_the_same_point() {
        assert_eq!(distance_sq(p(0, 0), p(3, 4)), 25);
        assert_eq!(distance_sq(p(3, 4), p(0, 0)), 25);
        assert_eq!(distance_sq(p(7, 7), p(7, 7)), 0);
    }

    #[test]
    fn distance_does_not_overflow_on_huge_coordinates() {
        let d = distance_sq(p(i32::MIN / 2, 0), p(i32::MAX / 2, 0));
        assert!(d > 0);
    }
}
