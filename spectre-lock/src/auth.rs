use std::path::Path;

const SERVICES: [&str; 3] = ["spectre-lock", "system-auth", "login"];

pub fn user() -> String {
    if let Ok(name) = std::env::var("USER") {
        if !name.is_empty() {
            return name;
        }
    }
    if let Ok(name) = std::env::var("LOGNAME") {
        if !name.is_empty() {
            return name;
        }
    }
    from_passwd(&std::fs::read_to_string("/etc/passwd").unwrap_or_default(), uid())
        .unwrap_or_else(|| String::from("root"))
}

fn uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .as_deref()
        .and_then(own_uid)
        .unwrap_or(0)
}

pub fn own_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find(|line| line.starts_with("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

pub fn from_passwd(passwd: &str, uid: u32) -> Option<String> {
    for line in passwd.lines() {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _password = fields.next();
        let found: u32 = fields.next()?.parse().ok()?;
        if found == uid {
            return Some(name.to_owned());
        }
    }
    None
}

pub fn services_in(directory: &Path) -> Vec<&'static str> {
    SERVICES.iter().copied().filter(|name| directory.join(name).exists()).collect()
}

pub fn check(user: &str, password: &str) -> bool {
    if password.is_empty() {
        return false;
    }
    let services = services_in(Path::new("/etc/pam.d"));
    if services.is_empty() {
        tracing::error!("no PAM service to ask, so the password cannot be checked");
        return false;
    }
    for service in services {
        match ask_pam(service, user, password) {
            Ok(true) => return true,
            Ok(false) => tracing::info!(service, "the password was refused"),
            Err(err) => tracing::warn!(service, %err, "this PAM service could not be used"),
        }
    }
    false
}

fn ask_pam(service: &str, user: &str, password: &str) -> Result<bool, String> {
    let mut client = pam::Client::with_password(service).map_err(|err| err.to_string())?;
    client.conversation_mut().set_credentials(user, password);
    match client.authenticate() {
        Ok(()) => Ok(true),
        Err(err) => {
            tracing::debug!(service, %err, "PAM said no");
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_own_user_id_comes_out_of_the_status_file() {
        let status = "Name:\tspectre\nUid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(own_uid(status), Some(1000));
        assert_eq!(own_uid("Name:\tspectre\n"), None);
    }

    #[test]
    fn the_name_for_an_id_comes_out_of_the_passwd_file() {
        let passwd = "root:x:0:0::/root:/bin/bash\nleon:x:1000:1000::/home/leon:/usr/bin/fish\n";
        assert_eq!(from_passwd(passwd, 1000).as_deref(), Some("leon"));
        assert_eq!(from_passwd(passwd, 0).as_deref(), Some("root"));
        assert_eq!(from_passwd(passwd, 4711), None);
    }

    #[test]
    fn only_services_that_exist_are_asked() {
        let directory = std::env::temp_dir().join("spectre-lock-pam-test");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("system-auth"), "auth required pam_unix.so\n").unwrap();
        assert_eq!(services_in(&directory), vec!["system-auth"]);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn nothing_to_ask_means_no_service_at_all() {
        assert!(services_in(Path::new("/nowhere-at-all")).is_empty());
    }

    #[test]
    fn an_empty_password_is_never_right() {
        assert!(!check("nobody", ""));
    }
}
