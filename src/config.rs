//! Non-secret preferences in `$XDG_CONFIG_HOME/antarctical/settings.json`. Credentials are never
//! written here; see `secrets.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    #[default]
    Month,
    Week,
    Day,
    Agenda,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Calendars the user toggled explicitly; others follow Google's `selected` flag.
    pub calendar_visibility: BTreeMap<String, bool>,
    /// Accounts that have explicitly opted into RSVP and completed OAuth consent for it.
    pub rsvp_accounts: std::collections::BTreeSet<String>,
    pub reminders: bool,
    pub clock_24h: bool,
    pub view: ViewMode,
    /// Show events you declined (greyed and struck through). Off: they are hidden everywhere.
    pub show_declined: bool,
    /// Demo mode: never written to disk.
    #[serde(skip)]
    pub ephemeral: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            calendar_visibility: BTreeMap::new(),
            rsvp_accounts: std::collections::BTreeSet::new(),
            reminders: true,
            clock_24h: true,
            view: ViewMode::Month,
            show_declined: false,
            ephemeral: false,
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|p| p.join("antarctical"))
}

impl Prefs {
    pub fn load() -> Self {
        let Some(path) = config_dir().map(|d| d.join("settings.json")) else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
                eprintln!("antarctical: ignoring unreadable {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Atomic replace (write to a temp file, then rename).
    pub fn save(&self) {
        if self.ephemeral {
            return;
        }
        let Some(dir) = config_dir() else { return };
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&dir)?;
            let tmp = dir.join("settings.json.tmp");
            std::fs::write(
                &tmp,
                serde_json::to_vec_pretty(self)
                    .expect("Prefs contains only JSON-serializable settings fields"),
            )?;
            std::fs::rename(&tmp, dir.join("settings.json"))
        })();
        if let Err(e) = result {
            eprintln!("antarctical: could not save settings: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn unknown_and_missing_fields_fall_back_to_defaults() {
        let p: Prefs = serde_json::from_str(r#"{"view":"week","future":1}"#).unwrap();
        assert_eq!(p.view, ViewMode::Week);
        assert!(p.reminders);
        assert!(!p.show_declined);
    }

    #[test]
    fn rsvp_accounts_are_opt_in_and_persist_per_account() {
        assert!(Prefs::default().rsvp_accounts.is_empty());
        let p: Prefs =
            serde_json::from_str(r#"{"rsvp_accounts":["me@example.com"],"future":1}"#).unwrap();
        assert_eq!(p.rsvp_accounts, BTreeSet::from(["me@example.com".into()]));
    }
}
