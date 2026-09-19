use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smithay_client_toolkit::reexports::calloop::channel::Sender as LoopSender;
use spectre_text::Image;

pub const ICON_SIZE: u32 = 18;

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";
const LOOK_AGAIN: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub service: String,
    pub path: String,
    pub title: String,
    pub icon: Option<Image>,
    pub letter: String,
}

#[derive(Debug)]
pub enum Command {
    Activate { service: String, path: String, at: (i32, i32) },
    Context { service: String, path: String, at: (i32, i32) },
}

pub fn start(updates: LoopSender<Vec<Item>>) -> Option<Sender<Command>> {
    let (commands, orders) = std::sync::mpsc::channel();
    let known: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let watcher = Watcher { known: known.clone() };

    let connection = zbus::blocking::connection::Builder::session()
        .ok()?
        .name(WATCHER_NAME)
        .ok()?
        .serve_at(WATCHER_PATH, watcher)
        .ok()?
        .build();
    let connection = match connection {
        Ok(connection) => connection,
        Err(err) => {
            tracing::info!(?err, "another program already holds the system tray");
            return None;
        }
    };

    std::thread::spawn(move || tend_the_tray(connection, known, orders, updates));
    Some(commands)
}

struct Watcher {
    known: Arc<Mutex<Vec<String>>>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: String) {
        let Ok(mut known) = self.known.lock() else {
            return;
        };
        if !known.contains(&service) {
            tracing::info!(%service, "a program put an icon in the tray");
            known.push(service);
        }
    }

    fn register_status_notifier_host(&self, service: String) {
        tracing::debug!(%service, "another tray host said hello");
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.known.lock().map(|known| known.clone()).unwrap_or_default()
    }
}

fn tend_the_tray(
    connection: zbus::blocking::Connection,
    known: Arc<Mutex<Vec<String>>>,
    orders: Receiver<Command>,
    updates: LoopSender<Vec<Item>>,
) {
    let mut shown: Vec<Item> = Vec::new();
    loop {
        match orders.recv_timeout(LOOK_AGAIN) {
            Ok(Command::Activate { service, path, at }) => {
                call(&connection, &service, &path, "Activate", at);
            }
            Ok(Command::Context { service, path, at }) => {
                call(&connection, &service, &path, "ContextMenu", at);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        let services = known.lock().map(|known| known.clone()).unwrap_or_default();
        let mut gone = Vec::new();
        let mut items = Vec::new();
        for service in services {
            match read_item(&connection, &service) {
                Some(item) => items.push(item),
                None => gone.push(service),
            }
        }
        if !gone.is_empty() {
            if let Ok(mut known) = known.lock() {
                known.retain(|service| !gone.contains(service));
            }
        }
        if items != shown {
            shown = items.clone();
            if updates.send(items).is_err() {
                return;
            }
        }
    }
}

fn call(
    connection: &zbus::blocking::Connection,
    service: &str,
    path: &str,
    method: &str,
    at: (i32, i32),
) {
    let proxy = zbus::blocking::Proxy::<'_>::new(connection, service, path, ITEM_INTERFACE);
    let Ok(proxy) = proxy else {
        return;
    };
    if let Err(err) = proxy.call_method(method, &at) {
        tracing::debug!(?err, %service, %method, "the tray icon did not take the click");
    }
}

fn read_item(connection: &zbus::blocking::Connection, service: &str) -> Option<Item> {
    let (name, path) = split_service(service);
    let title: String;
    let id: String;
    let pixmaps: Vec<(i32, i32, Vec<u8>)>;
    {
        let proxy = zbus::blocking::Proxy::<'_>::new(
            connection,
            name.as_str(),
            path.as_str(),
            ITEM_INTERFACE,
        )
        .ok()?;
        title = proxy.get_property("Title").unwrap_or_default();
        id = proxy.get_property("Id").unwrap_or_default();
        pixmaps = proxy.get_property("IconPixmap").unwrap_or_default();
    }
    if title.is_empty() && id.is_empty() {
        return None;
    }
    let icon = best_pixmap(&pixmaps, ICON_SIZE)
        .and_then(|(width, height, bytes)| from_argb(width, height, bytes))
        .map(|image| scaled(&image, ICON_SIZE));

    let shown = match title.is_empty() {
        true => id.clone(),
        false => title,
    };
    Some(Item { service: name, path, letter: short_name(&shown), title: shown, icon })
}

pub fn split_service(service: &str) -> (String, String) {
    match service.split_once('/') {
        Some((name, rest)) => (name.to_owned(), format!("/{rest}")),
        None => (service.to_owned(), String::from(DEFAULT_ITEM_PATH)),
    }
}

pub fn short_name(title: &str) -> String {
    title
        .chars()
        .find(|letter| letter.is_alphanumeric())
        .map(|letter| letter.to_uppercase().to_string())
        .unwrap_or_else(|| String::from("?"))
}

pub fn best_pixmap(
    pixmaps: &[(i32, i32, Vec<u8>)],
    wanted: u32,
) -> Option<(u32, u32, &[u8])> {
    let mut best: Option<(u32, u32, &[u8])> = None;
    let mut best_distance = u32::MAX;
    for (width, height, bytes) in pixmaps {
        if *width <= 0 || *height <= 0 {
            continue;
        }
        let (width, height) = (*width as u32, *height as u32);
        if bytes.len() < (width * height * 4) as usize {
            continue;
        }
        let distance = width.abs_diff(wanted);
        let better = distance < best_distance || (distance == best_distance && width > wanted);
        if better {
            best_distance = distance;
            best = Some((width, height, bytes.as_slice()));
        }
    }
    best
}

pub fn from_argb(width: u32, height: u32, bytes: &[u8]) -> Option<Image> {
    let wanted = (width as usize) * (height as usize) * 4;
    if width == 0 || height == 0 || bytes.len() < wanted {
        return None;
    }
    let mut data = vec![0u8; wanted];
    for pixel in 0..(width as usize * height as usize) {
        let at = pixel * 4;
        let alpha = bytes[at] as u32;
        let premultiplied = |value: u8| ((value as u32 * alpha) / 255) as u8;
        data[at] = premultiplied(bytes[at + 1]);
        data[at + 1] = premultiplied(bytes[at + 2]);
        data[at + 2] = premultiplied(bytes[at + 3]);
        data[at + 3] = alpha as u8;
    }
    Some(Image { width, height, data })
}

pub fn scaled(image: &Image, size: u32) -> Image {
    if size == 0 || image.is_empty() {
        return Image::empty();
    }
    if image.width == size && image.height == size {
        return image.clone();
    }
    let mut data = vec![0u8; size as usize * size as usize * 4];
    let step_x = image.width as f32 / size as f32;
    let step_y = image.height as f32 / size as f32;
    for y in 0..size {
        for x in 0..size {
            let from_x = (x as f32 * step_x).floor() as u32;
            let from_y = (y as f32 * step_y).floor() as u32;
            let to_x = ((x + 1) as f32 * step_x).ceil().min(image.width as f32) as u32;
            let to_y = ((y + 1) as f32 * step_y).ceil().min(image.height as f32) as u32;

            let mut sums = [0u32; 4];
            let mut count = 0u32;
            for source_y in from_y..to_y.max(from_y + 1) {
                for source_x in from_x..to_x.max(from_x + 1) {
                    if source_x >= image.width || source_y >= image.height {
                        continue;
                    }
                    let at = (source_y as usize * image.width as usize + source_x as usize) * 4;
                    for (channel, sum) in sums.iter_mut().enumerate() {
                        *sum += image.data[at + channel] as u32;
                    }
                    count += 1;
                }
            }
            let Some(count) = std::num::NonZeroU32::new(count) else {
                continue;
            };
            let out = (y as usize * size as usize + x as usize) * 4;
            for (channel, sum) in sums.iter().enumerate() {
                data[out + channel] = (sum / count.get()) as u8;
            }
        }
    }
    Image { width: size, height: size, data }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_without_a_path_gets_the_usual_one() {
        let (name, path) = split_service("org.kde.StatusNotifierItem-4711-1");
        assert_eq!(name, "org.kde.StatusNotifierItem-4711-1");
        assert_eq!(path, "/StatusNotifierItem");
    }

    #[test]
    fn a_service_can_carry_its_own_path() {
        let (name, path) = split_service(":1.42/org/ayatana/NotificationItem/nm_applet");
        assert_eq!(name, ":1.42");
        assert_eq!(path, "/org/ayatana/NotificationItem/nm_applet");
    }

    #[test]
    fn the_letter_stands_in_for_a_missing_icon() {
        assert_eq!(short_name("Steam"), "S");
        assert_eq!(short_name("  discord"), "D");
        assert_eq!(short_name("!!!"), "?");
        assert_eq!(short_name(""), "?");
    }

    #[test]
    fn the_pixmap_closest_to_the_wanted_size_wins() {
        let small = (16, 16, vec![255u8; 16 * 16 * 4]);
        let large = (64, 64, vec![255u8; 64 * 64 * 4]);
        let pixmaps = vec![small, large];
        let (width, _, _) = best_pixmap(&pixmaps, 18).expect("a pixmap");
        assert_eq!(width, 16);
        let (width, _, _) = best_pixmap(&pixmaps, 50).expect("a pixmap");
        assert_eq!(width, 64);
    }

    #[test]
    fn a_pixmap_that_is_too_short_is_refused() {
        let broken = vec![(16, 16, vec![0u8; 10])];
        assert!(best_pixmap(&broken, 18).is_none());
        assert!(from_argb(16, 16, &[0u8; 10]).is_none());
        assert!(from_argb(0, 0, &[]).is_none());
    }

    #[test]
    fn argb_from_dbus_becomes_premultiplied_rgba() {
        let bytes = vec![128, 255, 0, 0];
        let image = from_argb(1, 1, &bytes).expect("an image");
        assert_eq!(image.data, vec![128, 0, 0, 128], "half see-through red");
    }

    #[test]
    fn scaling_keeps_a_solid_colour_solid() {
        let image = Image { width: 4, height: 4, data: vec![200u8; 4 * 4 * 4] };
        let small = scaled(&image, 2);
        assert_eq!(small.width, 2);
        assert_eq!(small.height, 2);
        assert!(small.data.iter().all(|byte| *byte == 200));
    }

    #[test]
    fn scaling_to_the_same_size_hands_the_picture_back() {
        let image = Image { width: 3, height: 3, data: vec![7u8; 3 * 3 * 4] };
        assert_eq!(scaled(&image, 3), image);
        assert!(scaled(&image, 0).is_empty());
    }
}
