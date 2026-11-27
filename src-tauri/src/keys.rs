//! API keys and sign-in tokens, kept only in the operating system's keychain (macOS
//! Keychain, Windows Credential Manager, the Secret Service on Linux).
//!
//! Frameloft never writes them anywhere else. When the keychain can't be used, saving fails
//! with an explanation instead of quietly falling back to the catalogue.

use anyhow::{bail, Result};
use rusqlite::Connection;
use std::path::Path;

const KEYRING_SERVICE: &str = "Lumio Photo Manager";

/// The names keys are stored under.
pub const NAMES: &[&str] = &["anthropic", "openai", "smugmug"];

fn help() -> &'static str {
    if cfg!(target_os = "macos") {
        "Open Keychain Access and make sure your login keychain is unlocked, then try again."
    } else if cfg!(target_os = "windows") {
        "Check that Windows Credential Manager is working for your account, then try again."
    } else {
        "Frameloft needs a Secret Service keyring (GNOME Keyring or KWallet), installed and unlocked. Start or unlock it, then try again."
    }
}

pub fn get(which: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, which).ok()?.get_password().ok().filter(|k| !k.is_empty())
}

/// Stores `value` (or removes the key when it is empty).
pub fn set(which: &str, value: &str) -> Result<()> {
    if !NAMES.contains(&which) {
        bail!("unknown key {which}");
    }
    let entry = keyring::Entry::new(KEYRING_SERVICE, which);
    let res = entry.and_then(|e| {
        if value.is_empty() {
            match e.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(x) => Err(x),
            }
        } else {
            e.set_password(value)
        }
    });
    if let Err(x) = res {
        log::warn!("keychain unavailable: {x}");
        bail!("Frameloft couldn't save this in your system keychain ({x}). Nothing was saved. {}", help());
    }
    Ok(())
}

/// Lumio (Frameloft's old name) 1.x kept keys in the catalogue when the keychain failed. Move any such key into
/// the keychain, and remove it from the catalogue either way (overwriting the old bytes),
/// so the person re-enters it if the keychain still can't take it.
pub fn migrate_plaintext(conn: &Connection) -> Result<usize> {
    let rows: Vec<(String, String)> = {
        let mut st = conn.prepare("SELECT key, value FROM settings WHERE key LIKE 'key\\_%' ESCAPE '\\'")?;
        let r = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        r.filter_map(|x| x.ok()).collect()
    };
    if rows.is_empty() {
        return Ok(0);
    }
    for (k, v) in &rows {
        let which = k.trim_start_matches("key_");
        match set(which, v) {
            Ok(()) => log::info!("moved the {which} key from the catalogue into the keychain"),
            Err(e) => log::warn!("removed the {which} key from the catalogue; it has to be entered again ({e})"),
        }
    }
    conn.execute_batch("PRAGMA secure_delete = ON;")?;
    conn.execute("DELETE FROM settings WHERE key LIKE 'key\\_%' ESCAPE '\\'", [])?;
    // Push the change out of the write-ahead log too.
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    Ok(rows.len())
}

/// Only the owner may read the catalogue and the log (Unix; Windows profiles are private).
pub fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.exists() {
            if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
                log::warn!("could not restrict {}: {e}", path.display());
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_names_are_refused() {
        assert!(set("../../etc", "x").is_err());
        assert!(set("smugmug2", "x").is_err());
    }

    #[test]
    fn plaintext_keys_leave_the_catalogue() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate(&conn).unwrap();
        crate::db::set_setting(&conn, "key_anthropic", "sk-test").unwrap();
        crate::db::set_setting(&conn, "keyboard_layout", "keep").unwrap();
        assert_eq!(migrate_plaintext(&conn).unwrap(), 1);
        assert_eq!(crate::db::get_setting(&conn, "key_anthropic"), None);
        // A setting that merely starts with "key" (no underscore) is not a key.
        assert_eq!(crate::db::get_setting(&conn, "keyboard_layout").as_deref(), Some("keep"));
        assert_eq!(migrate_plaintext(&conn).unwrap(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn restrict_makes_files_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("catalogue.sqlite");
        std::fs::write(&f, "x").unwrap();
        restrict(&f);
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
    }
}
