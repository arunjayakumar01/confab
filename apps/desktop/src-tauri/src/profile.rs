//! A profile is one Google account: a directory under the app data dir with its
//! token store, cache and device id. `PRISM_PROFILE` (default `default`) names the
//! one active at launch; `accounts` keeps the others side by side.

use confab_proto::chat::UserInfo;
use confab_proto::Endpoints;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub struct Profile {
    pub name: String,
    /// `<app_data_dir>/<profile>/`
    pub dir: PathBuf,
    pub endpoints: Endpoints,
    /// Random per profile, persisted in `dir/device_id`; sent in the relay `hello`.
    pub device_id: String,
}

impl Profile {
    /// The profile `PRISM_PROFILE` names, `default` when unset: the account active at launch.
    pub fn initial_name() -> String {
        Self::sanitize(&std::env::var("PRISM_PROFILE").unwrap_or_default())
    }

    /// A directory-safe profile name: ASCII letters, digits, `-`, `_` and `.`, never
    /// only dots (so it cannot climb out of the data dir); `default` when nothing is left.
    pub fn sanitize(raw: &str) -> String {
        let name: String = raw
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect();
        let name = name.trim_matches('.');
        if name.is_empty() {
            "default".to_string()
        } else {
            name.to_string()
        }
    }

    pub fn load(app_data_dir: PathBuf, name: &str) -> Result<Self, String> {
        let name = Self::sanitize(name);
        let dir = app_data_dir.join(&name);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let device_id = load_or_create_device_id(&dir)?;
        Ok(Self {
            name,
            dir,
            endpoints: Endpoints::from_env(),
            device_id,
        })
    }

    /// Real Google mode is detected by `PRISM_CHAT_API` containing `googleapis.com`.
    pub fn real_google(&self) -> bool {
        self.endpoints.chat_api.contains("googleapis.com")
    }

    pub fn db_path(&self) -> PathBuf {
        self.dir.join("cache.db")
    }

    pub fn token_file(&self) -> PathBuf {
        self.dir.join("auth.json")
    }

    /// `dir/account.json`: who is signed in here, for the account list.
    pub fn account_file(&self) -> PathBuf {
        self.dir.join("account.json")
    }
}

/// `account.json`: what the account list shows without a network call, written
/// whenever the signed-in user is known.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
}

impl AccountInfo {
    pub fn from_user(info: &UserInfo) -> Self {
        Self {
            email: info.email.clone(),
            display_name: info.name.clone(),
            photo: info.picture.clone(),
        }
    }

    pub fn read(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

fn load_or_create_device_id(dir: &std::path::Path) -> Result<String, String> {
    let path = dir.join("device_id");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim().to_string();
        if !existing.is_empty() {
            return Ok(existing);
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::write(&path, &id).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_persists_per_profile_dir() {
        let dir = std::env::temp_dir().join(format!("prism-profile-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = load_or_create_device_id(&dir).unwrap();
        let b = load_or_create_device_id(&dir).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 36);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_names_are_directory_safe() {
        assert_eq!(Profile::sanitize(""), "default");
        assert_eq!(Profile::sanitize("  two "), "two");
        assert_eq!(Profile::sanitize("acct-3"), "acct-3");
        assert_eq!(Profile::sanitize("../etc"), "etc");
        assert_eq!(Profile::sanitize(".."), "default");
        assert_eq!(Profile::sanitize("a/b c"), "abc");
    }

    #[test]
    fn account_file_round_trips_in_the_profile_dir() {
        let root =
            std::env::temp_dir().join(format!("prism-account-test-{}", uuid::Uuid::new_v4()));
        let p = Profile::load(root.clone(), "acct-1").unwrap();
        assert_eq!(p.name, "acct-1");
        assert_eq!(p.dir, root.join("acct-1"));
        assert_eq!(p.account_file(), root.join("acct-1").join("account.json"));
        assert!(AccountInfo::read(&p.account_file()).is_none());
        let info = AccountInfo::from_user(&UserInfo {
            sub: "1002".into(),
            email: Some("priya@prism.test".into()),
            name: Some("Priya Nair".into()),
            picture: Some("http://t/twin/avatars/1002.svg".into()),
            hd: None,
        });
        info.write(&p.account_file()).unwrap();
        assert_eq!(AccountInfo::read(&p.account_file()), Some(info.clone()));
        let text = std::fs::read_to_string(p.account_file()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["displayName"], "Priya Nair");
        assert_eq!(v["email"], "priya@prism.test");
        assert_eq!(v["photo"], "http://t/twin/avatars/1002.svg");
        let _ = std::fs::remove_dir_all(&root);
    }
}
