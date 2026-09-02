//! The category column of the application menu.
//!
//! Categories come from the freedesktop main categories in a `.desktop` file's
//! `Categories=` key. Anything that matches none of them lands in `Other`, so
//! every installed application is reachable from some category.

use crate::entry::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    All,
    Development,
    Graphics,
    Internet,
    Multimedia,
    Office,
    Games,
    System,
    Settings,
    Utilities,
    Other,
}

impl Category {
    pub const ALL: [Category; 11] = [
        Category::All,
        Category::Development,
        Category::Graphics,
        Category::Internet,
        Category::Multimedia,
        Category::Office,
        Category::Games,
        Category::System,
        Category::Settings,
        Category::Utilities,
        Category::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::All => "All Applications",
            Category::Development => "Development",
            Category::Graphics => "Graphics",
            Category::Internet => "Internet",
            Category::Multimedia => "Multimedia",
            Category::Office => "Office",
            Category::Games => "Games",
            Category::System => "System",
            Category::Settings => "Settings",
            Category::Utilities => "Utilities",
            Category::Other => "Other",
        }
    }

    /// The `Categories=` tokens that put an entry in this category.
    fn tokens(self) -> &'static [&'static str] {
        match self {
            Category::All => &[],
            Category::Development => &["Development"],
            Category::Graphics => &["Graphics"],
            Category::Internet => &["Network"],
            Category::Multimedia => &["AudioVideo", "Audio", "Video"],
            Category::Office => &["Office"],
            Category::Games => &["Game"],
            Category::System => &["System"],
            Category::Settings => &["Settings"],
            Category::Utilities => &["Utility"],
            Category::Other => &[],
        }
    }

    pub fn contains(self, entry: &Entry) -> bool {
        match self {
            Category::All => true,
            Category::Other => !Self::ALL
                .iter()
                .filter(|c| !matches!(c, Category::All | Category::Other))
                .any(|c| c.contains(entry)),
            _ => {
                let wanted = self.tokens();
                entry.categories.split(';').any(|t| wanted.contains(&t.trim()))
            }
        }
    }
}

/// The entries in `category`, keeping the order they came in.
pub fn filter<'a>(category: Category, entries: &'a [Entry]) -> Vec<&'a Entry> {
    entries.iter().filter(|e| category.contains(e)).collect()
}

/// Categories that actually have something in them, plus `All`.
pub fn populated(entries: &[Entry]) -> Vec<Category> {
    Category::ALL
        .into_iter()
        .filter(|c| *c == Category::All || entries.iter().any(|e| c.contains(e)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, categories: &str) -> Entry {
        Entry {
            name: name.to_owned(),
            comment: String::new(),
            exec: String::from("true"),
            terminal: false,
            keywords: String::new(),
            categories: categories.to_owned(),
            id: name.to_owned(),
        }
    }

    #[test]
    fn an_entry_lands_in_the_category_its_desktop_file_names() {
        let gimp = entry("GIMP", "Graphics;2DGraphics;RasterGraphics;");
        assert!(Category::Graphics.contains(&gimp));
        assert!(!Category::Office.contains(&gimp));
        assert!(Category::All.contains(&gimp));
    }

    #[test]
    fn the_web_is_internet_rather_than_network_jargon() {
        let browser = entry("Firefox", "Network;WebBrowser;");
        assert!(Category::Internet.contains(&browser));
    }

    #[test]
    fn an_uncategorised_entry_is_reachable_under_other() {
        let odd = entry("Thing", "");
        assert!(Category::Other.contains(&odd));
        assert!(Category::All.contains(&odd));
    }

    #[test]
    fn an_entry_is_never_both_categorised_and_other() {
        let gimp = entry("GIMP", "Graphics;");
        assert!(!Category::Other.contains(&gimp));
    }

    #[test]
    fn a_partial_token_does_not_match() {
        let e = entry("Thing", "Networking;");
        assert!(!Category::Internet.contains(&e));
    }

    #[test]
    fn empty_categories_are_left_out_of_the_column() {
        let entries = vec![entry("GIMP", "Graphics;")];
        let shown = populated(&entries);
        assert!(shown.contains(&Category::All));
        assert!(shown.contains(&Category::Graphics));
        assert!(!shown.contains(&Category::Office));
    }

    #[test]
    fn filtering_keeps_the_incoming_order() {
        let entries = vec![entry("A", "Graphics;"), entry("B", "Office;"), entry("C", "Graphics;")];
        let names: Vec<_> =
            filter(Category::Graphics, &entries).iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["A", "C"]);
    }
}
