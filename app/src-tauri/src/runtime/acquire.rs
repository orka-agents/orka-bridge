use super::image_types::{ImageError, ImagePin};
use reqwest::{Client, Url};
use std::time::Duration;

pub struct ImageHttpClient {
    client: Client,
    pin: ImagePin,
    origins: Vec<String>,
}
fn secure_url(value: &str) -> Result<Url, ImageError> {
    let url = Url::parse(value).map_err(|_| ImageError::Invalid("image URL"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(ImageError::Invalid("image URL policy"));
    }
    let host = url
        .host_str()
        .ok_or(ImageError::Invalid("image host"))?
        .trim_end_matches('.');
    let loopback = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| {
        ip.is_loopback() || matches!(ip, std::net::IpAddr::V6(v) if v.to_ipv4_mapped().is_some_and(|v| v.is_loopback()))
    });
    if loopback || host == "localhost" || host.ends_with(".localhost") {
        return Err(ImageError::Invalid("loopback image origin"));
    }
    Ok(url)
}
fn client_builder() -> reqwest::ClientBuilder {
    Client::builder()
        .tls_backend_native()
        .https_only(true)
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
}
impl ImageHttpClient {
    pub async fn manifest(
        &self,
        closed: tokio::sync::watch::Receiver<bool>,
    ) -> Result<Vec<u8>, ImageError> {
        let mut bytes = Vec::new();
        self.transfer(
            &self.pin.manifest_url,
            &mut bytes,
            8 * 1024 * 1024,
            (None, &self.pin.manifest_sha256),
            closed,
            |_| {},
        )
        .await?;
        Ok(bytes)
    }
    async fn transfer<W: tokio::io::AsyncWrite + Unpin + Send>(
        &self,
        url: &str,
        output: &mut W,
        maximum: u64,
        expected: (Option<u64>, &str),
        mut closed: tokio::sync::watch::Receiver<bool>,
        progress: impl Fn(u64) + Send + Sync,
    ) -> Result<(), ImageError> {
        use sha2::{Digest, Sha256};
        use tokio::io::AsyncWriteExt;
        let cancelled = async |closed: &mut tokio::sync::watch::Receiver<bool>| loop {
            if *closed.borrow() || closed.changed().await.is_err() {
                break;
            }
        };
        let io_error = |e: std::io::Error| {
            if e.kind() == std::io::ErrorKind::StorageFull {
                ImageError::NoSpace
            } else {
                ImageError::Storage(e.kind())
            }
        };
        let mut url = secure_url(url)?;
        for hop in 0..=5 {
            if !self.origins.contains(&url.origin().ascii_serialization()) {
                return Err(ImageError::Invalid("image redirect origin"));
            }
            let response = tokio::select! {
                biased;
                _ = cancelled(&mut closed) => return Err(ImageError::Cancelled),
                result = self.client.get(url.clone()).header(reqwest::header::ACCEPT_ENCODING, "identity").send() =>
                    result.map_err(|_| ImageError::Network("image request"))?,
            };
            if response.status().is_redirection() {
                if hop == 5 {
                    return Err(ImageError::Network("image redirect limit"));
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or(ImageError::Network("image redirect location"))?;
                url = secure_url(
                    url.join(location)
                        .map_err(|_| ImageError::Invalid("image redirect URL"))?
                        .as_str(),
                )?;
                continue;
            }
            if response.status() != reqwest::StatusCode::OK {
                return Err(ImageError::Network("image status"));
            }
            if response
                .headers()
                .get_all(reqwest::header::CONTENT_ENCODING)
                .iter()
                .any(|v| !v.as_bytes().eq_ignore_ascii_case(b"identity"))
            {
                return Err(ImageError::Integrity);
            }
            if let Some(length) = response.content_length() {
                if length > maximum {
                    return Err(ImageError::LimitExceeded);
                }
                if expected.0.is_some_and(|wanted| length != wanted) {
                    return Err(ImageError::Integrity);
                }
            }
            let mut response = response;
            let mut bytes = 0u64;
            let mut hash = Sha256::new();
            loop {
                let chunk = tokio::select! {
                    biased;
                    _ = cancelled(&mut closed) => return Err(ImageError::Cancelled),
                    result = response.chunk() => result.map_err(|_| ImageError::Network("image body"))?,
                };
                let Some(chunk) = chunk else { break };
                bytes = bytes
                    .checked_add(chunk.len() as u64)
                    .filter(|n| *n <= maximum)
                    .ok_or(ImageError::LimitExceeded)?;
                output.write_all(&chunk).await.map_err(io_error)?;
                hash.update(&chunk);
                progress(bytes);
            }
            if *closed.borrow() || closed.has_changed().is_err() {
                return Err(ImageError::Cancelled);
            }
            if expected.0.is_some_and(|n| n != bytes)
                || format!("{:x}", hash.finalize()) != expected.1
            {
                return Err(ImageError::Integrity);
            }
            output.flush().await.map_err(io_error)?;
            return Ok(());
        }
        unreachable!("bounded redirect loop returns on its final hop")
    }
    pub fn new(pin: &ImagePin) -> Result<Self, ImageError> {
        let manifest = secure_url(&pin.manifest_url)?;
        let base = secure_url(&pin.asset_base)?;
        if !base.path().ends_with('/') || base.query().is_some() {
            return Err(ImageError::Invalid("asset base"));
        }
        let mut origins = vec![
            manifest.origin().ascii_serialization(),
            base.origin().ascii_serialization(),
        ];
        for origin in &pin.redirect_origins {
            let url = secure_url(origin)?;
            if url.path() != "/" || url.query().is_some() {
                return Err(ImageError::Invalid("redirect origin"));
            }
            origins.push(url.origin().ascii_serialization());
        }
        let client = client_builder()
            .build()
            .map_err(|_| ImageError::Unsupported)?;
        Ok(Self {
            client,
            pin: pin.clone(),
            origins,
        })
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ImageError> + Send + 'static,
) -> Result<T, ImageError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| ImageError::Storage(std::io::ErrorKind::Other))?
}
pub async fn acquire_image(
    image: &super::image_types::ResolvedImage,
    cache: &super::image_cache::ImageCache,
    client: &ImageHttpClient,
    closed: tokio::sync::watch::Receiver<bool>,
    progress: impl Fn(super::image_types::ImageProgress) + Send + Sync,
) -> Result<super::image_types::VerifiedImageSet, ImageError> {
    use super::image_types::ImageProgress;
    if client.pin.manifest_sha256 != image.identity().manifest_sha256 {
        return Err(ImageError::Invalid("image client pin"));
    }
    let (store, descriptor, stop) = (cache.clone(), image.clone(), closed.clone());
    progress(ImageProgress::Verifying);
    if let Some(set) = blocking(move || store.verified_checked(&descriptor, Some(&stop))).await? {
        progress(ImageProgress::Verified);
        return Ok(set);
    }
    let (store, descriptor, stop) = (cache.clone(), image.clone(), closed.clone());
    let tx = std::sync::Arc::new(
        blocking(move || {
            super::image_cache::verification::cancelled(&stop)?;
            store.begin(&descriptor)
        })
        .await?,
    );
    let total = image.assets().iter().try_fold(0u64, |n, a| {
        n.checked_add(a.bytes()).ok_or(ImageError::LimitExceeded)
    })?;
    let mut done = 0;
    progress(ImageProgress::Download { bytes: done, total });
    for (index, asset) in image.assets().iter().enumerate() {
        let (owner, stop) = (tx.clone(), closed.clone());
        let pending = blocking(move || {
            super::image_cache::verification::cancelled(&stop)?;
            owner.start_asset(index)
        })
        .await?;
        if let Some(file) = pending {
            let mut file = tokio::fs::File::from_std(file);
            let url = format!("{}{}", asset.location().base(), asset.location().leaf());
            client
                .transfer(
                    &url,
                    &mut file,
                    asset.bytes(),
                    (Some(asset.bytes()), asset.sha256()),
                    closed.clone(),
                    |bytes| {
                        progress(ImageProgress::Download {
                            bytes: done + bytes,
                            total,
                        })
                    },
                )
                .await?;
            drop(file);
            let (owner, stop) = (tx.clone(), closed.clone());
            blocking(move || {
                super::image_cache::verification::cancelled(&stop)?;
                owner.finish_asset(index)
            })
            .await?;
        }
        done += asset.bytes();
        progress(ImageProgress::Download { bytes: done, total });
    }
    progress(ImageProgress::Verifying);
    let tx = std::sync::Arc::try_unwrap(tx).map_err(|_| ImageError::Busy)?;
    let set = blocking(move || tx.commit(&closed)).await?;
    progress(ImageProgress::Verified);
    Ok(set)
}

#[cfg(test)]
#[path = "image_http_tests.rs"]
mod tests;
