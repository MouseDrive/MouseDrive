#![deny(unsafe_code)]

use minisign_verify::{PublicKey, Signature};
use mousedrive::log;

use super::error::UpdateError;
use super::parse_version;

const RELEASE_PUBLIC_KEY: &str = "";
const ALLOW_LEGACY: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Policy {
    WarnOnly,
    Require(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verified {
    Signed,
    Unchecked,
}

pub(super) fn policy() -> Policy {
    policy_for(RELEASE_PUBLIC_KEY)
}

fn policy_for(key: &'static str) -> Policy {
    if key.trim().is_empty() {
        Policy::WarnOnly
    } else {
        Policy::Require(key.trim())
    }
}

pub(super) fn check(
    policy: Policy,
    sums: &[u8],
    fetch: impl FnOnce() -> Result<String, UpdateError>,
    version: &str,
) -> Result<Verified, UpdateError> {
    match policy {
        Policy::WarnOnly => {
            log::warn("güncelleme imzası denetlenmedi: yayın anahtarı henüz tanımlı değil");
            Ok(Verified::Unchecked)
        }
        Policy::Require(key) => {
            verify(key, sums, &fetch()?, version)?;
            log::info(&format!("SHA256SUMS imzası doğrulandı ({version})"));
            Ok(Verified::Signed)
        }
    }
}

fn verify(key: &str, sums: &[u8], signature: &str, version: &str) -> Result<(), UpdateError> {
    let key = PublicKey::from_base64(key)
        .map_err(|e| UpdateError::Signature(format!("yayın anahtarı okunamadı: {e}")))?;
    let signature = Signature::decode(signature)
        .map_err(|e| UpdateError::Signature(format!("imza dosyası okunamadı: {e}")))?;
    key.verify(sums, &signature, ALLOW_LEGACY)
        .map_err(|e| UpdateError::Signature(e.to_string()))?;
    let comment = signature.trusted_comment();
    if mentions_version(comment, version) {
        Ok(())
    } else {
        Err(UpdateError::Signature(format!(
            "güvenilir yorum {version} sürümünü içermiyor: {comment}"
        )))
    }
}

fn mentions_version(comment: &str, version: &str) -> bool {
    let Some(wanted) = parse_version(version) else {
        return false;
    };
    comment
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .map(|token| token.trim_matches('.'))
        .any(|token| parse_version(token) == Some(wanted))
}
