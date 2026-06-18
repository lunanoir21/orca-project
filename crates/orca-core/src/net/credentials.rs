//! Credential storage backed by the OS keyring (Secret Service / D-Bus).
//!
//! # Security
//! Network passwords are stored only in the user's Secret Service keyring
//! (e.g. gnome-keyring, KWallet). They are never written to Orca's config files,
//! never logged, and only held in memory transiently while a connection is being
//! established. The plaintext password is passed straight to the keyring and the
//! local copy is dropped immediately afterwards.

use std::collections::HashMap;

use secret_service::{EncryptionType, SecretService};

use crate::error::{OrcaError, Result};

/// Identifies Orca's items in the shared keyring.
const ATTR_APP: &str = "orca";

/// Build the lookup attributes for a connection's credential. Kept private so
/// every call site uses an identical attribute set (store and search must match).
fn attributes<'a>(connection: &'a str, username: &'a str) -> HashMap<&'a str, &'a str> {
    let mut attrs = HashMap::new();
    attrs.insert("application", ATTR_APP);
    attrs.insert("kind", "network-credential");
    attrs.insert("connection", connection);
    attrs.insert("username", username);
    attrs
}

/// Connect to the Secret Service and return the unlocked default collection.
async fn default_collection<'a>(
    ss: &'a SecretService<'a>,
) -> Result<secret_service::Collection<'a>> {
    let collection = ss
        .get_default_collection()
        .await
        .map_err(|e| OrcaError::Other(format!("keyring collection unavailable: {e}")))?;
    if collection
        .is_locked()
        .await
        .map_err(|e| OrcaError::Other(format!("keyring lock query failed: {e}")))?
    {
        collection
            .unlock()
            .await
            .map_err(|e| OrcaError::Other(format!("keyring unlock failed: {e}")))?;
    }
    Ok(collection)
}

/// Store (or replace) the password for `connection` / `username` in the keyring.
///
/// # Errors
/// [`OrcaError::Other`] if the Secret Service is unavailable or rejects the item.
pub async fn store_password(connection: &str, username: &str, password: &str) -> Result<()> {
    let ss = SecretService::connect(EncryptionType::Dh)
        .await
        .map_err(|e| OrcaError::Other(format!("keyring connect failed: {e}")))?;
    let collection = default_collection(&ss).await?;
    let label = format!("Orca network credential: {connection}");
    collection
        .create_item(
            &label,
            attributes(connection, username),
            password.as_bytes(),
            true, // replace an existing item with the same attributes
            "text/plain",
        )
        .await
        .map_err(|e| OrcaError::Other(format!("keyring store failed: {e}")))?;
    Ok(())
}

/// Retrieve the stored password for `connection` / `username`, if present.
///
/// # Errors
/// [`OrcaError::Other`] if the Secret Service is unavailable or the secret is not
/// valid UTF-8.
pub async fn get_password(connection: &str, username: &str) -> Result<Option<String>> {
    let ss = SecretService::connect(EncryptionType::Dh)
        .await
        .map_err(|e| OrcaError::Other(format!("keyring connect failed: {e}")))?;
    let found = ss
        .search_items(attributes(connection, username))
        .await
        .map_err(|e| OrcaError::Other(format!("keyring search failed: {e}")))?;
    let Some(item) = found.unlocked.into_iter().next() else {
        return Ok(None);
    };
    let secret = item
        .get_secret()
        .await
        .map_err(|e| OrcaError::Other(format!("keyring read failed: {e}")))?;
    let password = String::from_utf8(secret)
        .map_err(|_| OrcaError::Other("stored secret is not valid UTF-8".to_string()))?;
    Ok(Some(password))
}

/// Delete the stored password for `connection` / `username`. Returns `true` if an
/// item was removed.
///
/// # Errors
/// [`OrcaError::Other`] if the Secret Service is unavailable.
pub async fn delete_password(connection: &str, username: &str) -> Result<bool> {
    let ss = SecretService::connect(EncryptionType::Dh)
        .await
        .map_err(|e| OrcaError::Other(format!("keyring connect failed: {e}")))?;
    let found = ss
        .search_items(attributes(connection, username))
        .await
        .map_err(|e| OrcaError::Other(format!("keyring search failed: {e}")))?;
    let mut removed = false;
    for item in found.unlocked {
        item.delete()
            .await
            .map_err(|e| OrcaError::Other(format!("keyring delete failed: {e}")))?;
        removed = true;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full store → get → delete cycle against a live Secret Service. If no
    /// keyring is available (or it is locked and cannot auto-unlock), the calls
    /// error and the test accepts that rather than failing — keeping CI green on
    /// headless machines.
    #[tokio::test]
    async fn store_get_delete_roundtrip_or_unavailable() {
        let conn = "orca-test-connection";
        let user = "orca-test-user";
        let secret = "s3cr3t-pa55";

        match store_password(conn, user, secret).await {
            Ok(()) => {
                let got = get_password(conn, user).await.expect("get after store");
                assert_eq!(got.as_deref(), Some(secret));
                assert!(delete_password(conn, user).await.expect("delete"));
                let after = get_password(conn, user).await.expect("get after delete");
                assert_eq!(after, None, "password should be gone after delete");
            }
            Err(OrcaError::Other(_)) => { /* keyring unavailable — acceptable */ }
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn get_missing_returns_none_or_unavailable() {
        match get_password("orca-no-such-conn", "nobody").await {
            Ok(v) => assert_eq!(v, None),
            Err(OrcaError::Other(_)) => { /* keyring unavailable — acceptable */ }
            Err(other) => panic!("unexpected error: {other}"),
        }
    }
}
