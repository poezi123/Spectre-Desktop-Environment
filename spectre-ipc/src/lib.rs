pub mod protocol;

pub use protocol::{Desktop, Event, Mode, Output, Request, Window, WindowId, Workspace};

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

pub const SOCKET_ENV: &str = "SPECTRE_SOCKET";

pub fn socket_path(wayland_display: &str) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("spectre-{wayland_display}.sock"))
}

pub fn client_socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(SOCKET_ENV) {
        return Some(PathBuf::from(path));
    }
    let display = std::env::var("WAYLAND_DISPLAY").ok()?;
    Some(socket_path(&display))
}

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    pub fn connect() -> io::Result<Self> {
        let path = client_socket_path().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "neither SPECTRE_SOCKET nor WAYLAND_DISPLAY is set",
            )
        })?;
        Self::connect_to(&path)
    }

    pub fn connect_to(path: &std::path::Path) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        Ok(Self { reader: BufReader::new(stream.try_clone()?), writer: stream })
    }

    pub fn as_raw(&self) -> &UnixStream {
        &self.writer
    }

    pub fn send(&mut self, request: &Request) -> io::Result<()> {
        let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()
    }

    pub fn recv(&mut self) -> io::Result<Option<Event>> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        parse_line(&line).map(Some)
    }

    pub fn request_state(&mut self) -> io::Result<Option<Desktop>> {
        self.send(&Request::GetState)?;
        loop {
            match self.recv()? {
                Some(Event::State(desktop)) => return Ok(Some(desktop)),
                Some(Event::Error { message }) => {
                    return Err(io::Error::other(message));
                }
                Some(Event::ConfigChanged) => continue,
                None => return Ok(None),
            }
        }
    }
}

pub fn parse_line<T: serde::de::DeserializeOwned>(line: &str) -> io::Result<T> {
    serde_json::from_str(line.trim_end_matches(['\r', '\n']))
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

pub fn encode_line<T: serde::Serialize>(value: &T) -> io::Result<String> {
    let mut line = serde_json::to_string(value).map_err(io::Error::other)?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_name_follows_the_wayland_display() {
        let a = socket_path("wayland-1");
        let b = socket_path("wayland-2");
        assert_ne!(a, b);
        assert!(a.to_string_lossy().ends_with("spectre-wayland-1.sock"));
    }

    #[test]
    fn encoded_messages_are_exactly_one_line() {
        let line = encode_line(&Request::Subscribe).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
    }

    #[test]
    fn parsing_tolerates_both_line_endings() {
        let request: Request = parse_line("{\"request\":\"subscribe\"}\r\n").unwrap();
        assert_eq!(request, Request::Subscribe);
        let request: Request = parse_line("{\"request\":\"subscribe\"}").unwrap();
        assert_eq!(request, Request::Subscribe);
    }

    #[test]
    fn a_malformed_line_is_an_error_not_a_panic() {
        assert!(parse_line::<Request>("{").is_err());
        assert!(parse_line::<Request>("").is_err());
    }

    #[test]
    fn a_client_and_server_can_talk_over_a_real_socket() {
        use std::io::BufRead;
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join(format!("spectre-ipc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: Request = parse_line(&line).unwrap();
            assert_eq!(request, Request::GetState);

            let event = Event::State(Desktop { animations: true, ..Default::default() });
            let mut writer = stream;
            writer.write_all(encode_line(&event).unwrap().as_bytes()).unwrap();
        });

        let mut client = Client::connect_to(&path).unwrap();
        let desktop = client.request_state().unwrap().unwrap();
        assert!(desktop.animations);
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
