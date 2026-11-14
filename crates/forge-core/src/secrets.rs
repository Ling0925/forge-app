use forge_protocol::ProviderId;
use std::{collections::BTreeMap, sync::Mutex};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("secret store operation failed")]
pub struct SecretStoreError;

pub trait SecretStore: Send + Sync {
    fn get(&self, provider: &ProviderId) -> Result<Option<String>, SecretStoreError>;
    fn set(&self, provider: &ProviderId, key: &str) -> Result<(), SecretStoreError>;
    fn delete(&self, provider: &ProviderId) -> Result<(), SecretStoreError>;
}

#[derive(Default)]
pub struct MemorySecretStore {
    keys: Mutex<BTreeMap<ProviderId, String>>,
}
impl SecretStore for MemorySecretStore {
    fn get(&self, provider: &ProviderId) -> Result<Option<String>, SecretStoreError> {
        Ok(self.keys.lock().unwrap().get(provider).cloned())
    }
    fn set(&self, provider: &ProviderId, key: &str) -> Result<(), SecretStoreError> {
        self.keys
            .lock()
            .unwrap()
            .insert(provider.clone(), key.into());
        Ok(())
    }
    fn delete(&self, provider: &ProviderId) -> Result<(), SecretStoreError> {
        self.keys.lock().unwrap().remove(provider);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub struct KeychainSecretStore;
#[cfg(target_os = "macos")]
impl SecretStore for KeychainSecretStore {
    fn get(&self, provider: &ProviderId) -> Result<Option<String>, SecretStoreError> {
        match security_framework::passwords::get_generic_password("dev.forge.app", &provider.0) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| SecretStoreError),
            Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => {
                Ok(None)
            }
            Err(_) => Err(SecretStoreError),
        }
    }
    fn set(&self, provider: &ProviderId, key: &str) -> Result<(), SecretStoreError> {
        security_framework::passwords::set_generic_password(
            "dev.forge.app",
            &provider.0,
            key.as_bytes(),
        )
        .map_err(|_| SecretStoreError)
    }
    fn delete(&self, provider: &ProviderId) -> Result<(), SecretStoreError> {
        match security_framework::passwords::delete_generic_password("dev.forge.app", &provider.0) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == security_framework_sys::base::errSecItemNotFound => {
                Ok(())
            }
            Err(_) => Err(SecretStoreError),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn memory_keys_are_isolated_and_replaceable_and_deletable() {
        let store = MemorySecretStore::default();
        let id = "one".into();
        assert_eq!(store.get(&id).unwrap(), None);
        store.set(&id, "first").unwrap();
        assert_eq!(store.get(&id).unwrap(), Some("first".into()));
        assert_eq!(store.get(&"two".into()).unwrap(), None);
        store.set(&id, "second").unwrap();
        assert_eq!(store.get(&id).unwrap(), Some("second".into()));
        store.delete(&id).unwrap();
        assert_eq!(store.get(&id).unwrap(), None);
        store.delete(&id).unwrap();
    }
}
