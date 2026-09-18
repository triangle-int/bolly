//! OS-backed saved connection. The canonical origin and token are one native-owned
//! keychain record, so browser-writable metadata can never retarget a credential.
use serde::{Deserialize, Serialize};

const SERVICE: &str = "com.triangle-int.bolly-desktop.self-hosted";
const ACCOUNT: &str = "saved-connection-v2";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct SavedConnection {
    pub origin: String,
    pub token: String,
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|_| "OS credential store unavailable".into())
}

pub(crate) async fn store(connection: SavedConnection) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let value =
            serde_json::to_string(&connection).map_err(|_| "Could not encode saved connection")?;
        entry()?
            .set_password(&value)
            .map_err(|_| "Could not save connection in OS credential store".into())
    })
    .await
    .map_err(|_| "Credential operation failed")?
}

pub(crate) async fn read() -> Result<Option<SavedConnection>, String> {
    tauri::async_runtime::spawn_blocking(move || match entry()?.get_password() {
        Ok(value) => serde_json::from_str(&value)
            .map(Some)
            .map_err(|_| "Invalid saved connection".into()),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("Could not read connection from OS credential store".into()),
    })
    .await
    .map_err(|_| "Credential operation failed")?
}

pub(crate) async fn delete() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("Could not remove connection from OS credential store".into()),
    })
    .await
    .map_err(|_| "Credential operation failed")?
}

pub(crate) async fn delete_legacy(reference: String) -> Result<(), String> {
    if uuid::Uuid::parse_str(&reference).is_err() {
        return Ok(());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let entry = keyring::Entry::new(SERVICE, &reference)
            .map_err(|_| "OS credential store unavailable")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("Could not remove legacy credential".into()),
        }
    })
    .await
    .map_err(|_| "Credential operation failed")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_record_binds_origin_and_secret_together() {
        let record = SavedConnection {
            origin: "https://example.org:8443".into(),
            token: "TOP_SECRET".into(),
        };
        let encoded = serde_json::to_string(&record).unwrap();
        let decoded: SavedConnection = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.origin, "https://example.org:8443");
        assert_eq!(decoded.token, "TOP_SECRET");
    }
}
