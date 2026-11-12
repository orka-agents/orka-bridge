//! Native-owned source selection. A catalog entry is a reviewed release input,
//! never a renderer URL or a development environment override.
use crate::runtime::image_types::*;

#[derive(Clone)]
pub(super) struct ReleaseImage {
    pub pin: ImagePin,
    // Keep the reviewed manifest in the app profile so cache-free reopen can
    // recheck authenticated host/app policy without network or new disk state.
    pub manifest: Vec<u8>,
    pub image_id: String,
    pub state_compatibility_id: String,
}
impl ReleaseImage {
    fn validate_host(&self, host: &HostTarget) -> Result<ResolvedImage, ImageError> {
        let resolved = crate::runtime::manifest::resolve_image(&self.pin, &self.manifest, host)?;
        self.validate(resolved.identity(), resolved.target())?;
        Ok(resolved)
    }
    pub fn validate(
        &self,
        identity: &ImageIdentity,
        target: ImageTarget,
    ) -> Result<(), ImageError> {
        if identity.manifest_sha256 != self.pin.manifest_sha256
            || identity.image_id != self.image_id
            || identity.state_compatibility_id != self.state_compatibility_id
            || target != self.pin.target
        {
            return Err(ImageError::Integrity);
        }
        Ok(())
    }
}

/// No image is approved yet. Wiring startup must not fabricate a shipping pin.
pub(super) fn shipping_image(_target: ImageTarget) -> Option<ReleaseImage> {
    None
}

#[derive(Clone)]
pub(super) enum ImageSource {
    #[cfg(debug_assertions)]
    Development,
    Pinned {
        profile: Box<ReleaseImage>,
        host: HostTarget,
    },
}
impl ImageSource {
    pub fn compatibility(&self) -> &str {
        match self {
            #[cfg(debug_assertions)]
            Self::Development => crate::dev_image::STATE_COMPATIBILITY_ID,
            Self::Pinned { profile, .. } => &profile.state_compatibility_id,
        }
    }
    pub fn validate(
        &self,
        identity: &ImageIdentity,
        target: ImageTarget,
    ) -> Result<(), ImageError> {
        match self {
            #[cfg(debug_assertions)]
            Self::Development => Ok(()),
            Self::Pinned { profile, .. } => profile.validate(identity, target),
        }
    }
}

pub(super) fn select_source(
    development: bool,
    override_present: bool,
    installed: bool,
    release: Option<(ReleaseImage, HostTarget)>,
) -> Option<ImageSource> {
    #[cfg(debug_assertions)]
    if development && (override_present || (installed && release.is_none())) {
        return Some(ImageSource::Development);
    }
    let _ = (development, override_present, installed);
    release.map(|(profile, host)| ImageSource::Pinned {
        profile: Box::new(profile),
        host,
    })
}

pub(super) fn open_selected(
    source: &mut ImageSource,
    journal: &crate::runtime::journal::Journal,
) -> Result<
    std::sync::Arc<crate::runtime::image_install::InstalledImageSet>,
    crate::runtime::types::RuntimeError,
> {
    use crate::runtime::image_install::open_installed_image;
    let opened = open_installed_image(&journal.image_target(source.compatibility())?);
    #[cfg(debug_assertions)]
    if matches!(source, ImageSource::Pinned { .. })
        && matches!(opened, Err(ImageError::RetainedImage))
    {
        // A root's existence does not identify its source. Preserve committed
        // development reopen only when its verified binding has that identity.
        if let Ok(image) =
            open_installed_image(&journal.image_target(crate::dev_image::STATE_COMPATIBILITY_ID)?)
        {
            *source = ImageSource::Development;
            return Ok(std::sync::Arc::new(image));
        }
    }
    opened.map(std::sync::Arc::new).map_err(Into::into)
}

/// This helper never owns a backend. The factory keeps the original Journal
/// across preparation retries and transfers it only at the native launch boundary.
#[allow(clippy::too_many_arguments)] // Keep the source, paths and retained owner explicit.
pub(super) fn prepare_installation(
    source: &ImageSource,
    root: &std::path::Path,
    cache_path: &std::path::Path,
    custody: &mut crate::runtime::journal::JournalCustody,
    closed: tokio::sync::watch::Receiver<bool>,
    progress: impl Fn(crate::local_vm::LocalProgress) + Send + Sync,
    create: impl FnOnce()
        -> Result<crate::runtime::journal::Journal, crate::runtime::types::RuntimeError>,
    runtime: &tokio::runtime::Handle,
) -> Result<
    std::sync::Arc<crate::runtime::image_install::InstalledImageSet>,
    crate::runtime::types::RuntimeError,
> {
    use crate::{
        local_vm::{LocalProgress, LocalVmStatus},
        runtime::{acquire::ImageHttpClient, image_cache::ImageCache, types::RuntimeError},
    };
    if *closed.borrow() {
        return Err(RuntimeError::Closed);
    }
    progress(LocalProgress::Image(ImageProgress::Verifying));
    let mut source = source.clone();
    let existing = if let Some(journal) = &custody.0 {
        match open_selected(&mut source, journal) {
            Ok(image) => Some(image),
            Err(RuntimeError::Image(ImageError::RetainedImage)) => None,
            Err(error) => return Err(error),
        }
    } else {
        match super::reopen(root, &mut source)? {
            Some((journal, image)) => {
                custody.0 = Some(journal);
                Some(image)
            }
            None => None,
        }
    };
    let resolved = match &source {
        #[cfg(debug_assertions)]
        ImageSource::Development => None,
        ImageSource::Pinned { profile, host } => Some(profile.validate_host(host)?),
    };
    if let Some(image) = existing {
        source.validate(image.identity(), image.target())?;
        if let Some(resolved) = &resolved {
            image.validate_pinned(resolved)?;
        }
        return Ok(image);
    }
    let prepared = match &source {
        #[cfg(debug_assertions)]
        ImageSource::Development => {
            let image = crate::dev_image::from_environment_cancellable(&closed)
                .map_err(|_| {
                    if *closed.borrow() {
                        RuntimeError::Closed
                    } else {
                        RuntimeError::Unavailable
                    }
                })?
                .ok_or(RuntimeError::Unavailable)?;
            runtime.block_on(crate::runtime::setup_adapter::prepare_development_image(
                &image,
                closed.clone(),
            ))?
        }
        ImageSource::Pinned { profile, host } => {
            std::fs::create_dir_all(cache_path.parent().ok_or(RuntimeError::Unavailable)?)?;
            let cache = ImageCache::open(cache_path)?;
            let client = ImageHttpClient::new(&profile.pin)?;
            runtime.block_on(crate::runtime::setup_adapter::prepare_pinned_image(
                &profile.pin,
                host,
                &cache,
                &client,
                closed.clone(),
                |event| progress(LocalProgress::Image(event)),
            ))?
        }
    };
    source.validate(prepared.identity(), prepared.target())?;
    if *closed.borrow() {
        return Err(RuntimeError::Closed);
    }
    if custody.0.is_none() {
        custody.0 = Some(create()?);
    }
    progress(LocalProgress::Status(LocalVmStatus::Installing));
    let journal = custody.0.as_ref().ok_or(RuntimeError::Unavailable)?;
    let image = runtime.block_on(prepared.materialize(
        &journal.image_target(source.compatibility())?,
        closed,
        |event| progress(LocalProgress::Image(event)),
    ))?;
    if let Some(resolved) = &resolved {
        image.validate_pinned(resolved)?;
    }
    Ok(std::sync::Arc::new(image))
}

pub(super) fn checked_host_version(value: &str) -> Result<String, ImageError> {
    let value = value.strip_suffix('\0').unwrap_or(value);
    let parts: Vec<_> = value.split('.').collect();
    if !(2..=3).contains(&parts.len())
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || part.parse::<u32>().is_err()
        })
    {
        return Err(ImageError::Unsupported);
    }
    Ok(value.into())
}

pub(super) fn native_host_target(target: ImageTarget) -> Result<HostTarget, ImageError> {
    #[cfg(all(windows, target_arch = "x86_64"))]
    let version = {
        use windows_sys::{
            Wdk::System::SystemServices::RtlGetVersion,
            Win32::System::SystemInformation::OSVERSIONINFOW,
        };
        if target != ImageTarget::WindowsAmd64 {
            return Err(ImageError::Unsupported);
        }
        let mut info = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        // Unlike compatibility-manifest version APIs, this reports the host OS.
        if unsafe { RtlGetVersion(&mut info) } != 0 {
            return Err(ImageError::Unsupported);
        }
        format!(
            "{}.{}.{}",
            info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber
        )
    };
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let version = {
        if target != ImageTarget::MacosArm64 {
            return Err(ImageError::Unsupported);
        }
        let mut bytes = [0u8; 64];
        let mut size = bytes.len();
        // The product version, not Darwin's kernel version. Bounded and read-only.
        if unsafe {
            libc::sysctlbyname(
                c"kern.osproductversion".as_ptr(),
                bytes.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } != 0
            || size == 0
            || size > bytes.len()
            || bytes[size - 1] != 0
        {
            return Err(ImageError::Unsupported);
        }
        std::str::from_utf8(&bytes[..size])
            .map_err(|_| ImageError::Unsupported)?
            .to_owned()
    };
    #[cfg(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    ))]
    return Ok(HostTarget {
        target,
        host_version: checked_host_version(&version)?,
        app_version: env!("CARGO_PKG_VERSION").into(),
    });
    #[cfg(not(any(
        all(windows, target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )))]
    {
        let _ = target;
        Err(ImageError::Unsupported)
    }
}
