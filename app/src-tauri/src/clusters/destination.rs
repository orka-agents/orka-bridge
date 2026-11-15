//! Configured transport identity, not server or workload attestation. Never hash credentials.
use super::{AccessOutcome, Check};
use sha2::{Digest, Sha256};
use std::io::Read;

pub(super) fn fingerprint(config: &kube::Config) -> Result<String, AccessOutcome> {
    let failed = || Check::Configuration.unknown();
    let file = config
        .root_cert_file
        .as_ref()
        .map(|path| {
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|_| failed())?
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| failed())?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err(failed());
            }
            Ok(bytes)
        })
        .transpose()?;
    let bytes = serde_json::to_vec(&(
        config.cluster_url.to_string(),
        &config.root_cert,
        file,
        &config.tls_server_name,
        config.accept_invalid_certs,
    ))
    .map_err(|_| failed())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(crate) fn valid_identity(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_tracks_destination_and_trust_but_not_authentication() {
        let mut config = kube::Config::new("https://cluster.invalid/root".parse().unwrap());
        let identity = fingerprint(&config).unwrap();
        config.auth_info.token = Some("synthetic-not-an-identity".into());
        assert_eq!(fingerprint(&config).unwrap(), identity);
        for changed in [
            "https://other.invalid/root",
            "https://cluster.invalid/other",
        ] {
            let mut other = config.clone();
            other.cluster_url = changed.parse().unwrap();
            assert_ne!(fingerprint(&other).unwrap(), identity);
        }
        config.root_cert = Some(vec![vec![1, 2, 3]]);
        assert_ne!(fingerprint(&config).unwrap(), identity);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ca");
        std::fs::write(&path, "synthetic-ca-one").unwrap();
        config.root_cert_file = Some(path.clone());
        let first = fingerprint(&config).unwrap();
        std::fs::write(&path, "synthetic-ca-two").unwrap();
        assert_ne!(fingerprint(&config).unwrap(), first);
        std::fs::remove_file(path).unwrap();
        assert!(fingerprint(&config).is_err());
    }
}
