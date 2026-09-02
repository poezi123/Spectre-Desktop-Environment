//! The Spectre Wayland compositor.
//!
//! ```text
//! spectre-compositor [--backend winit|udev] [--config PATH] [--profile NAME]
//!                    [--command CMD]
//! ```

mod backend;
mod grabs;
mod handlers;
mod input;
mod ipc;
mod layout;
mod render;
mod transition;
mod state;
mod workspace;

use anyhow::{bail, Context};
use spectre_config::{Config, Profile};

use crate::backend::Backend;

fn main() -> anyhow::Result<()> {
    init_tracing();
    reap_children();

    let args = Args::parse(std::env::args().skip(1))?;
    if args.help {
        print!("{}", Args::USAGE);
        return Ok(());
    }

    if let Some(path) = &args.config {
        // Every component reads the same file, so the settings app edits what
        // the session is actually running.
        std::env::set_var(spectre_config::CONFIG_ENV, path);
    }
    let mut config = match &args.config {
        Some(path) => Config::load_from(path).with_context(|| {
            format!("failed to read the configuration at {}", path.display())
        })?,
        None => {
            let (config, error) = Config::load();
            if let Some(error) = error {
                tracing::error!(%error, "using built-in defaults");
            }
            config
        }
    };

    if let Some(profile) = args.profile {
        config.general.profile = profile;
        config = config.resolved();
    }
    config.general.autostart.extend(args.commands);

    let backend = args.backend.unwrap_or_else(Backend::detect);
    tracing::info!(
        backend = backend.name(),
        profile = config.general.profile.label(),
        workspaces = config.general.workspaces,
        "starting Spectre"
    );

    match backend {
        #[cfg(feature = "winit")]
        Backend::Winit => crate::backend::winit::run(config),
        #[cfg(not(feature = "winit"))]
        Backend::Winit => bail!("this build has no winit backend; rebuild with --features winit"),
        #[cfg(feature = "udev")]
        Backend::Udev => crate::backend::udev::run(config),
        #[cfg(not(feature = "udev"))]
        Backend::Udev => bail!("this build has no udev backend; rebuild with --features udev"),
    }
}

/// Let the kernel reap the processes the compositor starts.
///
/// A desktop spawns a lot of children and never waits on any of them, so
/// without this every launched application leaves a zombie behind. Ignoring
/// `SIGCHLD` makes the kernel clean them up; the compositor never calls
/// `wait`, so nothing is lost by doing so.
fn reap_children() {
    // SAFETY: setting a disposition on SIGCHLD is async-signal-safe and is
    // done once, before any thread or child exists.
    unsafe {
        libc::signal(libc::SIGCHLD, libc::SIG_IGN);
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("SPECTRE_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
}

/// Parsed command line.
#[derive(Debug, Default, PartialEq)]
struct Args {
    help: bool,
    backend: Option<Backend>,
    config: Option<std::path::PathBuf>,
    profile: Option<Profile>,
    /// Extra commands to start once the session is up.
    commands: Vec<String>,
}

impl Args {
    const USAGE: &'static str = "\
spectre-compositor - the Spectre Wayland compositor

USAGE:
    spectre-compositor [OPTIONS]

OPTIONS:
    -b, --backend <winit|udev>   Force a backend instead of detecting one
    -c, --config <PATH>          Read this configuration file
    -p, --profile <NAME>         performance | balanced | spectre | custom
    -e, --command <CMD>          Run CMD once the session is up (repeatable)
    -h, --help                   Show this help

ENVIRONMENT:
    SPECTRE_LOG                  Log filter, e.g. `debug` or `spectre=trace`
";

    fn parse(args: impl IntoIterator<Item = String>) -> anyhow::Result<Args> {
        let mut out = Args::default();
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            let mut value = |name: &str| -> anyhow::Result<String> {
                args.next().with_context(|| format!("{name} needs a value"))
            };
            match arg.as_str() {
                "-h" | "--help" => out.help = true,
                "-b" | "--backend" => {
                    let raw = value("--backend")?;
                    out.backend = Some(
                        Backend::parse(&raw)
                            .with_context(|| format!("unknown backend `{raw}`"))?,
                    );
                }
                "-c" | "--config" => out.config = Some(value("--config")?.into()),
                "-p" | "--profile" => {
                    let raw = value("--profile")?;
                    out.profile = Some(parse_profile(&raw)?);
                }
                "-e" | "--command" => out.commands.push(value("--command")?),
                other => bail!("unknown argument `{other}` (try --help)"),
            }
        }

        Ok(out)
    }
}

fn parse_profile(name: &str) -> anyhow::Result<Profile> {
    Profile::ALL
        .into_iter()
        .find(|p| p.label().eq_ignore_ascii_case(name))
        .with_context(|| format!("unknown profile `{name}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> anyhow::Result<Args> {
        Args::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_arguments_detects_everything() {
        let args = parse(&[]).unwrap();
        assert_eq!(args, Args::default());
    }

    #[test]
    fn long_and_short_flags_agree() {
        assert_eq!(parse(&["-b", "udev"]).unwrap(), parse(&["--backend", "udev"]).unwrap());
    }

    #[test]
    fn commands_accumulate_in_order() {
        let args = parse(&["-e", "waybar", "--command", "foot"]).unwrap();
        assert_eq!(args.commands, ["waybar", "foot"]);
    }

    #[test]
    fn a_missing_value_is_an_error_not_a_panic() {
        assert!(parse(&["--backend"]).is_err());
        assert!(parse(&["--config"]).is_err());
    }

    #[test]
    fn unknown_arguments_are_rejected() {
        assert!(parse(&["--nonsense"]).is_err());
        assert!(parse(&["-x"]).is_err());
    }

    #[test]
    fn profiles_parse_case_insensitively() {
        assert_eq!(parse_profile("PERFORMANCE").unwrap(), Profile::Performance);
        assert_eq!(parse_profile("spectre").unwrap(), Profile::Spectre);
        assert!(parse_profile("turbo").is_err());
    }
}
