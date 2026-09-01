//! Command line control for a running Spectre session.
//!
//! Small on purpose: it exists so the desktop can be inspected and driven from
//! a terminal without a panel, which is how the IPC gets tested.

use std::process::ExitCode;

use spectre_ipc::{Client, Event, Request};

const USAGE: &str = "\
spectrectl - control a running Spectre session

USAGE:
    spectrectl <COMMAND>

COMMANDS:
    state                 Print the desktop state as JSON
    watch                 Print the state, then every change
    workspace <N>         Switch to workspace N
    activate <ID>         Focus a window
    minimize <ID>         Minimize a window
    close <ID>            Ask a window to close
    profile <NAME>        performance | balanced | spectre | custom
    animations <on|off>   Flip the animation kill switch
    quit                  End the session
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("spectrectl: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        print!("{USAGE}");
        return Ok(());
    };
    if matches!(command, "-h" | "--help" | "help") {
        print!("{USAGE}");
        return Ok(());
    }

    let mut client = Client::connect().map_err(|err| {
        format!("cannot reach the compositor ({err}); is a Spectre session running?")
    })?;

    let request = match (command, args.get(1).map(String::as_str)) {
        ("state", _) => {
            let state = client
                .request_state()
                .map_err(|e| e.to_string())?
                .ok_or("the compositor closed the connection")?;
            println!("{}", serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?);
            return Ok(());
        }
        ("watch", _) => {
            client.send(&Request::Subscribe).map_err(|e| e.to_string())?;
            while let Some(event) = client.recv().map_err(|e| e.to_string())? {
                match event {
                    Event::State(state) => {
                        println!("{}", serde_json::to_string(&state).map_err(|e| e.to_string())?)
                    }
                    Event::Error { message } => eprintln!("compositor: {message}"),
                }
            }
            return Ok(());
        }
        ("workspace", Some(n)) => Request::SwitchWorkspace {
            index: n.parse().map_err(|_| format!("`{n}` is not a workspace number"))?,
        },
        ("activate", Some(id)) => Request::ActivateWindow { id: parse_id(id)? },
        ("minimize", Some(id)) => Request::MinimizeWindow { id: parse_id(id)? },
        ("close", Some(id)) => Request::CloseWindow { id: parse_id(id)? },
        ("profile", Some(name)) => Request::SetProfile { profile: parse_profile(name)? },
        ("animations", Some(value)) => Request::SetAnimations {
            enabled: match value {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                other => return Err(format!("`{other}` is not on or off")),
            },
        },
        ("quit", _) => Request::Quit,
        (cmd, None) => return Err(format!("`{cmd}` needs an argument (try --help)")),
        (cmd, _) => return Err(format!("unknown command `{cmd}` (try --help)")),
    };

    client.send(&request).map_err(|e| e.to_string())
}

fn parse_id(text: &str) -> Result<u64, String> {
    text.parse().map_err(|_| format!("`{text}` is not a window id"))
}

fn parse_profile(name: &str) -> Result<spectre_config::Profile, String> {
    spectre_config::Profile::ALL
        .into_iter()
        .find(|p| p.label().eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("unknown profile `{name}`"))
}
