//! Source preparation precedes Journal ownership; callers materialize under its target.

#[cfg(debug_assertions)]
pub async fn prepare_development_image(
    image: &crate::dev_image::VerifiedDevImage,
    closed: tokio::sync::watch::Receiver<bool>,
) -> Result<super::image_install::PreparedImage, super::image_types::ImageError> {
    cancelled(&closed)?;
    let image = image.try_clone().map_err(|_| ImageError::Integrity)?;
    blocking(move || {
        cancelled(&closed)?;
        let prepared = PreparedImage::from_development(image)?;
        cancelled(&closed)?;
        Ok(prepared)
    })
    .await
}
use super::{
    acquire::{acquire_image, blocking, ImageHttpClient},
    image_cache::{verification::cancelled, ImageCache},
    image_install::PreparedImage,
    image_types::*,
    manifest::resolve_image,
};
use tokio::sync::watch;

/// The pin and host facts come from native selection, never renderer input.
/// Backend-specific host checks belong to that native caller, before this operation.
pub async fn prepare_pinned_image(
    pin: &ImagePin,
    host: &HostTarget,
    cache: &ImageCache,
    client: &ImageHttpClient,
    closed: watch::Receiver<bool>,
    progress: impl Fn(ImageProgress) + Send + Sync,
) -> Result<PreparedImage, ImageError> {
    let (store, selected, stop) = (cache.clone(), pin.clone(), closed.clone());
    let raw = blocking(move || {
        cancelled(&stop)?;
        let raw = store.manifest(&selected)?;
        cancelled(&stop)?;
        Ok(raw)
    })
    .await?;
    let raw = match raw {
        Some(raw) => raw,
        None => client.manifest(closed.clone()).await?,
    };
    let (selected, host, stop) = (pin.clone(), host.clone(), closed.clone());
    let image = blocking(move || {
        cancelled(&stop)?;
        resolve_image(&selected, &raw, &host)
    })
    .await?;
    let verified = acquire_image(&image, cache, client, closed.clone(), progress).await?;
    blocking(move || {
        cancelled(&closed)?;
        PreparedImage::from_verified(verified)
    })
    .await
}
