use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub comment: String,
    pub exec: String,
    pub terminal: bool,
    pub keywords: String,
    pub categories: String,
    pub id: String,
}

impl Entry {
    pub fn parse(id: &str, contents: &str, desktops: &[String]) -> Option<Entry> {
        let group = desktop_entry_group(contents)?;
        let get = |key: &str| group.get(key).map(String::as_str).unwrap_or_default();

        if get("Type") != "Application" {
            return None;
        }
        if is_true(get("NoDisplay")) || is_true(get("Hidden")) {
            return None;
        }
        if !shown_in(get("OnlyShowIn"), get("NotShowIn"), desktops) {
            return None;
        }

        let name = get("Name").trim().to_owned();
        let exec = strip_field_codes(get("Exec"));
        if name.is_empty() || exec.trim().is_empty() {
            return None;
        }

        Some(Entry {
            name,
            comment: get("Comment").trim().to_owned(),
            exec,
            terminal: is_true(get("Terminal")),
            keywords: format!("{} {}", get("Keywords"), get("Categories")).trim().to_owned(),
            categories: get("Categories").trim().to_owned(),
            id: id.to_owned(),
        })
    }

    pub fn load_all() -> Vec<Entry> {
        let desktops = current_desktops();
        let mut by_id: HashMap<String, Entry> = HashMap::new();
        for dir in application_dirs() {
            collect_dir(&dir, &dir, &desktops, &mut by_id);
        }
        let mut entries: Vec<Entry> = by_id.into_values().collect();
        entries.sort_by_key(|a| a.name.to_lowercase());
        entries
    }
}

fn current_desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_else(|_| String::from("Spectre"))
        .split(':')
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty())
        .collect()
}

fn shown_in(only: &str, not: &str, desktops: &[String]) -> bool {
    let listed = |list: &str| {
        list.split(';')
            .map(|d| d.trim().to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .any(|d| desktops.contains(&d))
    };
    if !only.trim().is_empty() {
        return listed(only);
    }
    !listed(not)
}

fn application_dirs() -> Vec<PathBuf> {
    let dirs = xdg::BaseDirectories::new();
    let mut out = Vec::new();
    if let Some(home) = dirs.get_data_home() {
        out.push(home.join("applications"));
    }
    out.extend(dirs.get_data_dirs().into_iter().map(|d| d.join("applications")));
    out.reverse();
    out
}

fn collect_dir(root: &Path, dir: &Path, desktops: &[String], out: &mut HashMap<String, Entry>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for item in read.flatten() {
        let path = item.path();
        if path.is_dir() {
            collect_dir(root, &path, desktops, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let id = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('/', "-");
        if let Some(entry) = Entry::parse(&id, &contents, desktops) {
            out.insert(id, entry);
        }
    }
}

fn desktop_entry_group(contents: &str) -> Option<HashMap<String, String>> {
    let mut in_group = false;
    let mut fields = HashMap::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.contains('[') {
            continue;
        }
        fields.insert(key.to_owned(), value.trim().to_owned());
    }

    (!fields.is_empty()).then_some(fields)
}

fn is_true(value: &str) -> bool {
    value.eq_ignore_ascii_case("true")
}

pub fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some(_) => {}
            None => {}
        }
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spectre() -> Vec<String> {
        vec![String::from("spectre")]
    }

    #[test]
    fn an_entry_meant_for_another_desktop_is_left_out() {
        let file = "[Desktop Entry]\nType=Application\nName=System Settings\nExec=systemsettings\nOnlyShowIn=KDE;";
        assert!(Entry::parse("kde.desktop", file, &spectre()).is_none());
    }

    #[test]
    fn an_entry_that_excludes_us_is_left_out() {
        let file = "[Desktop Entry]\nType=Application\nName=Thing\nExec=thing\nNotShowIn=Spectre;GNOME;";
        assert!(Entry::parse("t.desktop", file, &spectre()).is_none());
    }

    #[test]
    fn an_entry_that_names_us_is_kept() {
        let file = "[Desktop Entry]\nType=Application\nName=Thing\nExec=thing\nOnlyShowIn=Spectre;";
        assert!(Entry::parse("t.desktop", file, &spectre()).is_some());
    }

    #[test]
    fn an_entry_with_no_desktop_restriction_is_kept() {
        let file = "[Desktop Entry]\nType=Application\nName=Thing\nExec=thing";
        assert!(Entry::parse("t.desktop", file, &spectre()).is_some());
    }

    const KONSOLE: &str = "\
[Desktop Entry]
Type=Application
Name=Konsole
Name[de]=Konsole Terminal
Comment=Terminal emulator
Exec=konsole %u
Icon=utilities-terminal
Categories=Qt;KDE;System;TerminalEmulator;

[Desktop Action new-window]
Name=New Window
Exec=konsole --new-tab
";

    #[test]
    fn a_normal_entry_parses() {
        let e = Entry::parse("org.kde.konsole.desktop", KONSOLE, &spectre()).unwrap();
        assert_eq!(e.name, "Konsole");
        assert_eq!(e.comment, "Terminal emulator");
        assert_eq!(e.exec, "konsole");
        assert!(!e.terminal);
        assert!(e.keywords.contains("TerminalEmulator"));
    }

    #[test]
    fn only_the_desktop_entry_group_is_read() {
        let e = Entry::parse("x", KONSOLE, &spectre()).unwrap();
        assert_eq!(e.exec, "konsole", "an action's Exec must not win");
        assert_eq!(e.name, "Konsole", "an action's Name must not win");
    }

    #[test]
    fn hidden_entries_are_left_out() {
        for flag in ["NoDisplay=true", "Hidden=true", "NoDisplay=True"] {
            let file = format!("[Desktop Entry]\nType=Application\nName=X\nExec=x\n{flag}\n");
            assert!(Entry::parse("x", &file, &spectre()).is_none(), "{flag}");
        }
    }

    #[test]
    fn non_applications_are_left_out() {
        let link = "[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.com\n";
        assert!(Entry::parse("x", link, &spectre()).is_none());
        let dir = "[Desktop Entry]\nType=Directory\nName=Games\n";
        assert!(Entry::parse("x", dir, &spectre()).is_none());
    }

    #[test]
    fn entries_without_a_name_or_command_are_left_out() {
        assert!(Entry::parse("x", "[Desktop Entry]\nType=Application\nExec=x\n", &spectre()).is_none());
        assert!(Entry::parse("x", "[Desktop Entry]\nType=Application\nName=X\n", &spectre()).is_none());
        assert!(
            Entry::parse("x", "[Desktop Entry]\nType=Application\nName=X\nExec=%f\n", &spectre()).is_none(),
            "an Exec that is nothing but a field code launches nothing"
        );
    }

    #[test]
    fn junk_input_is_rejected_rather_than_guessed() {
        assert!(Entry::parse("x", "", &spectre()).is_none());
        assert!(Entry::parse("x", "not a desktop file at all", &spectre()).is_none());
        assert!(Entry::parse("x", "[Other Group]\nName=X\n", &spectre()).is_none());
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let file = "# a comment\n\n[Desktop Entry]\n# another\nType=Application\nName=X\nExec=x\n";
        assert!(Entry::parse("x", file, &spectre()).is_some());
    }

    #[test]
    fn field_codes_are_removed() {
        assert_eq!(strip_field_codes("firefox %u"), "firefox");
        assert_eq!(strip_field_codes("gimp %U %f %F"), "gimp");
        assert_eq!(strip_field_codes("app -i %i -c %c"), "app -i -c");
    }

    #[test]
    fn an_escaped_percent_survives() {
        assert_eq!(strip_field_codes("printf 100%% -x"), "printf 100% -x");
    }

    #[test]
    fn a_trailing_percent_does_not_panic() {
        assert_eq!(strip_field_codes("weird %"), "weird");
    }

    #[test]
    fn terminal_applications_are_flagged() {
        let file = "[Desktop Entry]\nType=Application\nName=htop\nExec=htop\nTerminal=true\n";
        assert!(Entry::parse("x", file, &spectre()).unwrap().terminal);
    }
}
