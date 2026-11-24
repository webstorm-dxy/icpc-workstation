use crate::error::{AppError, AppResult};
use keyring::Entry;

const SERVICE: &str = "com.icpcworkstation.desktop";

pub fn set_secret(provider_id: &str, secret: &str) -> AppResult<()> {
    let entry = Entry::new(SERVICE, provider_id)?;
    entry.set_password(secret)?;
    Ok(())
}

pub fn get_secret(provider_id: &str) -> AppResult<Option<String>> {
    let entry = Entry::new(SERVICE, provider_id)?;
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(AppError {
            code: "E_KEYCHAIN".into(),
            message: "无法读取系统钥匙串中的 API Key".into(),
            detail: Some(err.to_string()),
            retryable: false,
        }),
    }
}

pub fn delete_secret(provider_id: &str) -> AppResult<()> {
    let entry = Entry::new(SERVICE, provider_id)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(AppError {
            code: "E_KEYCHAIN".into(),
            message: "无法删除系统钥匙串中的 API Key".into(),
            detail: Some(err.to_string()),
            retryable: false,
        }),
    }
}
