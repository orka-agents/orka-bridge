//! App-level custody of one delivered native session; not a second VM lifecycle.
use crate::{
    engine::{types::*, EngineTransport, TransportFuture},
    runtime::{
        client::VmClient,
        types::{RuntimeError, WorkspaceFailure, WorkspaceOperation},
    },
};
use serde::Serialize;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::sync::{watch, Mutex as AsyncMutex};
#[path = "local_vm_native.rs"]
pub(crate) mod native;
pub use native::{configured, upgrade_target};
/// Opaque Rust-owned target. No serde constructor or renderer-selected paths.
pub struct UpgradeTarget {
    pub(crate) source: native::image::ImageSource,
    pub(crate) machine_id: Vec<u8>,
}
#[derive(Serialize)]
pub struct ExamplePair {
    pub agent: &'static str,
    pub tests: &'static str,
}
pub fn example() -> ExamplePair {
    ExamplePair {
        agent: include_str!("../example-agent/web-reader.agent.md"),
        tests: include_str!("../example-agent/web-reader.agent.test.md"),
    }
}

// Both supported platforms now use the paired model-capable guest images.
pub const MODEL_IMAGE_AVAILABLE: bool = crate::catalog::IMAGE_AVAILABLE;

pub async fn setup_status(
    store: Arc<crate::setup::SetupStore>,
    vm: &LocalVm,
) -> Result<crate::setup::SetupSnapshot, crate::setup::SetupError> {
    Ok(vm.snapshot(store.status_async().await?))
}
pub async fn select_local(
    store: Arc<crate::setup::SetupStore>,
    vm: &Arc<LocalVm>,
) -> Result<crate::setup::SetupSnapshot, crate::setup::SetupError> {
    let _choice = vm.choice.lock().await;
    let selected = store.select_local_async().await?;
    vm.start();
    Ok(vm.snapshot(selected))
}
pub async fn cancel_local(
    store: Arc<crate::setup::SetupStore>,
    vm: &Arc<LocalVm>,
) -> Result<crate::setup::SetupSnapshot, crate::setup::SetupError> {
    vm.cancel()
        .await
        .map_err(|_| crate::setup::SetupError::model("modelSession"))?;
    setup_status(store, vm).await
}
pub async fn change_local(
    store: Arc<crate::setup::SetupStore>,
    vm: &Arc<LocalVm>,
) -> Result<crate::setup::SetupSnapshot, crate::setup::SetupError> {
    let vm = vm.clone();
    // The confirmed change owns stop + preference removal even if IPC is dropped.
    tokio::spawn(async move {
        let _choice = vm.choice.lock().await;
        vm.cancel()
            .await
            .map_err(|_| crate::setup::SetupError::model("modelSession"))?;
        Ok(vm.snapshot(store.clear_local_async().await?))
    })
    .await
    .map_err(|_| crate::setup::SetupError::model("modelSession"))?
}
pub async fn resume_selected(
    store: Arc<crate::setup::SetupStore>,
    vm: &Arc<LocalVm>,
) -> Result<(), crate::setup::SetupError> {
    let _choice = vm.choice.lock().await;
    if store.status_async().await?.selected.is_some() {
        vm.start();
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalVmStatus {
    NotIntegrated,
    Idle,
    Preparing,
    Installing,
    Starting,
    Ready,
    Closing,
    Closed,
    Cancelled,
    Failed,
}
impl LocalVmStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotIntegrated => "notIntegrated",
            Self::Idle => "idle",
            Self::Preparing => "preparing",
            Self::Installing => "installing",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Closing => "closing",
            Self::Closed => "closed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}
fn failure_reason(error: &RuntimeError) -> String {
    match error {
        RuntimeError::Preflight(failure) => failure.summary().to_owned(),
        RuntimeError::Io(error) if error.kind() == std::io::ErrorKind::StorageFull => {
            format!("NoSpace: {error}")
        }
        RuntimeError::Io(error) => error.to_string(),
        RuntimeError::StopUnconfirmed(detail) => format!("startupStopFailed: {detail}"),
        // Already formatted when retained, including any NoSpace prefix.
        RuntimeError::Startup(context) => context.diagnostic.clone(),
        _ => error.to_string(),
    }
}

pub use crate::setup::{WorkspacePhase, WorkspaceProgress};
#[derive(Clone, Copy, Debug)]
pub enum LocalProgress {
    Status(LocalVmStatus),
    Image(crate::runtime::image_types::ImageProgress),
}
pub type StartFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn LocalSession>, RuntimeError>> + Send + 'a>>;
pub trait LocalFactory: Send + Sync {
    fn launch(
        &self,
        closed: watch::Receiver<bool>,
        progress: Arc<dyn Fn(LocalProgress) + Send + Sync>,
    ) -> StartFuture<'_>;
    /// Drop retained factory handles only after native stop and all queued work joined.
    fn release_stopped(&self) -> Result<(), RuntimeError> {
        Ok(())
    }
    /// Hand a just-created destination's live, new-only creation authority to the
    /// factory that will eventually launch it. Only the process that called
    /// `WorkspaceUpgrade::replace`/`retry` may call this; a later ordinary start
    /// must reuse this exact journal, never reopen the directory from scratch.
    /// Default: no native upgrade backend (debug/dev/unavailable factories).
    fn adopt_destination(
        &self,
        _journal: crate::runtime::journal::Journal,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::Unavailable)
    }
}
pub trait LocalSession: Send {
    /// From the installed guest build metadata, never a probing guest request.
    fn run_cancel_supported(&self) -> bool {
        false
    }
    /// Only called under the whole-session blocking custody; no client transfer.
    fn with_client(
        &mut self,
        _work: &mut dyn FnMut(
            bridge_runtime_protocol::Uuid,
            &mut VmClient,
        ) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::Unavailable)
    }
    fn execute(&mut self, request: EngineRequest) -> TransportFuture<'_>;
    fn stop(&mut self) -> Result<(), RuntimeError>;
}
struct State {
    phase: LocalVmStatus,
    model_connected: bool,
    run_cancel_supported: bool,
    failure: Option<String>,
    preflight_failure: Option<crate::setup::preflight::Failure>,
    workspace_failure: Option<WorkspaceFailure>,
    /// Latest typed progress, captured before failure clears the progress clock.
    operation: WorkspaceOperation,
    closing: bool,
    close_started: bool,
    stopped: Option<bool>,
    startup: Option<tokio::task::JoinHandle<bool>>,
    terminal: bool,
    cancelled: watch::Sender<bool>,
    completion: watch::Sender<Option<bool>>,
    progress: Option<ProgressClock>,
}
struct ProgressClock {
    started: std::time::Instant,
    phase: WorkspacePhase,
    bytes: Option<u64>,
    total: Option<u64>,
    sample: Option<(std::time::Instant, u64)>,
    remaining: Option<u64>,
}
impl ProgressClock {
    fn observe(&mut self, bytes: u64, total: u64, retained: bool, now: std::time::Instant) {
        let advanced = self.bytes != Some(bytes);
        self.bytes = Some(bytes);
        self.total = Some(total);
        self.remaining = None;
        if retained || self.sample.is_none() {
            self.sample = Some((now, bytes));
        } else if let Some((time, previous)) = self.sample {
            // Average fresh movement over the transfer, not a single 64 KiB chunk.
            // A quarter-second sample avoids sub-millisecond events erasing the ETA.
            let nanos = now.saturating_duration_since(time).as_nanos();
            if advanced && bytes > previous && nanos >= 250_000_000 && bytes < total {
                self.remaining = u64::try_from(
                    (total - bytes) as u128 * nanos / (bytes - previous) as u128 / 1_000_000,
                )
                .ok();
            }
        }
    }
    fn snapshot(&self) -> WorkspaceProgress {
        WorkspaceProgress {
            phase: self.phase,
            bytes: self.bytes,
            total_bytes: self.total,
            elapsed_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            remaining_ms: self.remaining,
        }
    }
}
#[cfg(test)]
mod progress_tests {
    use super::*;
    #[test]
    fn full_disk_diagnostic_keeps_the_actionable_category() {
        let error = RuntimeError::Io(std::io::ErrorKind::StorageFull.into());
        assert!(failure_reason(&error).starts_with("NoSpace: "));
    }
    #[test]
    fn startup_snapshot_has_native_elapsed_without_fake_bytes_or_eta() {
        let vm = LocalVm::unavailable();
        vm.inner
            .publish(LocalProgress::Status(LocalVmStatus::Starting));
        let mut state = vm.inner.state.lock().unwrap();
        let progress = state
            .progress
            .as_mut()
            .expect("startup has a monotonic clock");
        progress.started = std::time::Instant::now() - std::time::Duration::from_secs(42);
        let snapshot = serde_json::to_value(progress.snapshot()).unwrap();
        assert_eq!(snapshot["phase"], "starting");
        assert!(snapshot["elapsedMs"].as_u64().unwrap() >= 42000);
        assert!(
            snapshot["bytes"].is_null()
                && snapshot["totalBytes"].is_null()
                && snapshot["remainingMs"].is_null()
        );
    }
    #[test]
    fn eta_stays_useful_for_sub_millisecond_copy_events() {
        let now = std::time::Instant::now();
        let total = 3 * 1024 * 1024 * 1024u64;
        let mut clock = ProgressClock {
            started: now,
            phase: WorkspacePhase::Materializing,
            bytes: None,
            total: None,
            sample: None,
            remaining: None,
        };
        clock.observe(0, total, true, now);
        for i in 1..=20_000u64 {
            clock.observe(
                i * 65536,
                total,
                false,
                now + std::time::Duration::from_micros(i * 160 + 4000),
            );
        }
        let remaining = clock
            .remaining
            .expect("a sustained fast copy has a measured ETA");
        assert!((4600..=4800).contains(&remaining), "{remaining}");
    }
    #[test]
    fn eta_uses_fresh_movement_not_retained_bytes_or_stalled_samples() {
        let now = std::time::Instant::now();
        let mut clock = ProgressClock {
            started: now,
            phase: WorkspacePhase::Materializing,
            bytes: None,
            total: None,
            sample: None,
            remaining: None,
        };
        clock.observe(800, 1000, true, now);
        assert!(clock.remaining.is_none());
        clock.observe(
            850,
            1000,
            false,
            now + std::time::Duration::from_millis(500),
        );
        assert_eq!(clock.remaining, Some(1500));
        clock.observe(
            950,
            1000,
            true,
            now + std::time::Duration::from_millis(1000),
        );
        assert!(clock.remaining.is_none());
        clock.observe(
            975,
            1000,
            false,
            now + std::time::Duration::from_millis(1500),
        );
        assert_eq!(clock.remaining, Some(500));
        clock.observe(
            975,
            1000,
            false,
            now + std::time::Duration::from_millis(2000),
        );
        assert!(clock.remaining.is_none());
        clock.observe(
            1000,
            1000,
            false,
            now + std::time::Duration::from_millis(2500),
        );
        assert!(clock.remaining.is_none());
    }
}
#[cfg(test)]
#[path = "local_vm_model_tests.rs"]
mod model_tests;

struct Inner {
    state: Mutex<State>,
    session: Arc<AsyncMutex<Option<Box<dyn LocalSession>>>>,
}
impl Inner {
    fn model_reset_result(
        &self,
        result: &Result<crate::setup::ModelSetupState, crate::setup::SetupError>,
    ) {
        if result.is_ok() {
            let mut state = self.state.lock().unwrap();
            state.model_connected = false;
            state.failure = None;
        }
    }
    fn model_result(
        &self,
        result: &Result<crate::setup::ModelSetupState, crate::setup::SetupError>,
    ) {
        let mut state = self.state.lock().unwrap();
        if state.phase != LocalVmStatus::Ready || state.closing {
            return;
        }
        state.model_connected = result.as_ref().is_ok_and(|model| model.ready);
        match result {
            Ok(model) if model.ready => state.failure = None,
            Err(error) => {
                // A generic retry refusal must not erase the original guest diagnostic.
                if let Some(detail) = error.model_detail() {
                    state.failure = Some(detail);
                }
            }
            _ => {}
        }
    }
    fn publish(&self, progress: LocalProgress) {
        use crate::runtime::image_types::ImageProgress;
        let mut state = self.state.lock().unwrap();
        if state.closing {
            return;
        }
        match progress {
            LocalProgress::Status(phase) => {
                state.phase = phase;
                match phase {
                    LocalVmStatus::Installing => state.operation = WorkspaceOperation::Prepare,
                    LocalVmStatus::Starting => state.operation = WorkspaceOperation::Start,
                    _ => {}
                }
                if phase == LocalVmStatus::Starting {
                    state.progress = Some(ProgressClock {
                        started: std::time::Instant::now(),
                        phase: WorkspacePhase::Starting,
                        bytes: None,
                        total: None,
                        sample: None,
                        remaining: None,
                    });
                } else if !matches!(phase, LocalVmStatus::Preparing | LocalVmStatus::Installing) {
                    state.progress = None;
                }
            }
            LocalProgress::Image(event) => {
                let (phase, count) = match event {
                    ImageProgress::Verifying => (WorkspacePhase::Verifying, None),
                    ImageProgress::Materializing { bytes, total } => {
                        (WorkspacePhase::Materializing, Some((bytes, total, false)))
                    }
                    ImageProgress::Retained { bytes, total } => {
                        (WorkspacePhase::Materializing, Some((bytes, total, true)))
                    }
                    _ => return,
                };
                state.operation = match phase {
                    WorkspacePhase::Verifying => WorkspaceOperation::Verify,
                    _ => WorkspaceOperation::Prepare,
                };
                let now = std::time::Instant::now();
                let clock = state.progress.get_or_insert(ProgressClock {
                    started: now,
                    phase,
                    bytes: None,
                    total: None,
                    sample: None,
                    remaining: None,
                });
                if clock.phase != phase {
                    clock.phase = phase;
                    clock.sample = None;
                    clock.remaining = None;
                }
                if let Some((bytes, total, retained)) = count {
                    clock.observe(bytes, total, retained, now);
                }
            }
        }
    }
    fn fail(
        &self,
        reason: String,
        error: &RuntimeError,
        operation: WorkspaceOperation,
        stop_unconfirmed: bool,
    ) {
        let mut state = self.state.lock().unwrap();
        state.failure = Some(reason);
        state.preflight_failure = match error {
            RuntimeError::Preflight(failure) => Some(*failure),
            _ => None,
        };
        state.workspace_failure = Some(WorkspaceFailure::from_error(error, operation));
        state.progress = None;
        if !state.closing {
            state.phase = LocalVmStatus::Failed;
        }
        state.closing |= stop_unconfirmed;
    }
    fn ready(&self) -> bool {
        let state = self.state.lock().unwrap();
        !state.closing && state.phase == LocalVmStatus::Ready
    }
}
pub struct LocalVm {
    inner: Arc<Inner>,
    factory: Option<Arc<dyn LocalFactory>>,
    /// Only a fresh recovery-blocked owner proves factory custody was never acquired.
    recovery_without_factory: bool,
    recovery_blocked: std::sync::atomic::AtomicBool,
    choice: Arc<AsyncMutex<()>>,
    authoring: Mutex<Arc<AsyncMutex<()>>>,
    upgrading: std::sync::atomic::AtomicBool,
    /// Latches true once this process observes any upgrade-backed destination
    /// or an interrupted/restored transaction. Never cleared within a process:
    /// suppressing automatic first-test replay must survive a later successful
    /// status poll, a cleared failure or a later Restored transition.
    workspace_replaced: std::sync::atomic::AtomicBool,
    /// Last successfully computed status, so a concurrent poll during a busy
    /// mutation still shows real fields with `busy` overlaid, not a blank DTO.
    upgrade_cache: Mutex<Option<crate::workspace_upgrade::UpgradeStatusDto>>,
    workspace_replaced_observer: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Present only while a replace/retry/restore/transfer/remove is in
    /// flight; cooperative, matching the existing `closed` receiver contract.
    upgrade_cancel: Mutex<Option<watch::Sender<bool>>>,
}
/// Keep native launch admission and setup choice serialized through the switch.
pub(crate) struct UpgradeQuiescence<'a> {
    vm: &'a LocalVm,
    custody_released: bool,
    _choice: tokio::sync::OwnedMutexGuard<()>,
}
impl Drop for UpgradeQuiescence<'_> {
    fn drop(&mut self) {
        if self.custody_released {
            self.vm
                .upgrading
                .store(false, std::sync::atomic::Ordering::Release);
        } else {
            self.vm.quarantine_upgrade();
        }
    }
}
impl LocalVm {
    fn make(factory: Option<Arc<dyn LocalFactory>>, recovery_without_factory: bool) -> Arc<Self> {
        let (cancelled, _) = watch::channel(false);
        let (completion, _) = watch::channel(None);
        Arc::new(Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    phase: if factory.is_some() {
                        LocalVmStatus::Idle
                    } else {
                        LocalVmStatus::NotIntegrated
                    },
                    model_connected: false,
                    run_cancel_supported: false,
                    failure: None,
                    preflight_failure: None,
                    workspace_failure: None,
                    operation: WorkspaceOperation::Verify,
                    closing: false,
                    close_started: false,
                    stopped: None,
                    startup: None,
                    terminal: false,
                    cancelled,
                    completion,
                    progress: None,
                }),
                session: Arc::new(AsyncMutex::new(None)),
            }),
            factory,
            recovery_without_factory,
            recovery_blocked: std::sync::atomic::AtomicBool::new(false),
            choice: Arc::new(AsyncMutex::new(())),
            authoring: Mutex::new(Arc::new(AsyncMutex::new(()))),
            upgrading: std::sync::atomic::AtomicBool::new(false),
            workspace_replaced: std::sync::atomic::AtomicBool::new(false),
            upgrade_cache: Mutex::new(None),
            workspace_replaced_observer: Mutex::new(None),
            upgrade_cancel: Mutex::new(None),
        })
    }
    pub fn new(factory: Arc<dyn LocalFactory>) -> Arc<Self> {
        Self::make(Some(factory), false)
    }
    pub fn unavailable() -> Arc<Self> {
        Self::make(None, false)
    }
    pub fn recovery_blocked() -> Arc<Self> {
        let vm = Self::make(None, true);
        vm.block_for_recovery();
        vm
    }
    /// Retain the configured factory for explicit recovery without resuming startup.
    pub fn block_for_recovery(&self) {
        self.recovery_blocked
            .store(true, std::sync::atomic::Ordering::Release);
        self.set_workspace_replaced(true);
        self.inner.fail(
            "Workspace recovery is required before starting the local cluster.".into(),
            &RuntimeError::RetainedState,
            WorkspaceOperation::Verify,
            false,
        );
    }
    #[cfg(test)]
    pub(crate) async fn quiesce_for_upgrade(&self) -> Result<UpgradeQuiescence<'_>, RuntimeError> {
        let choice = self.choice.clone().lock_owned().await;
        self.quiesce_with_choice(choice).await
    }
    /// Reserve choice before preflight, then retain it through stop and handoff.
    /// A timeout keeps the gate and journal custody quarantined until process exit.
    async fn quiesce_with_choice(
        &self,
        choice: tokio::sync::OwnedMutexGuard<()>,
    ) -> Result<UpgradeQuiescence<'_>, RuntimeError> {
        self.upgrading
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(dto) = self.upgrade_cache.lock().unwrap().as_mut() {
            dto.busy = true;
            dto.error = None;
        }
        let mut guard = UpgradeQuiescence {
            vm: self,
            custody_released: false,
            _choice: choice,
        };
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(30), self.stop(false))
            .await
            .unwrap_or_else(|_| {
                Err(RuntimeError::StopUnconfirmed(
                    "Workspace stop did not complete.".into(),
                ))
            });
        stopped?;
        let released = match self.factory.as_ref() {
            Some(factory) => factory.release_stopped(),
            // Only this constructor proves no factory ever acquired handles.
            None if self.recovery_without_factory => Ok(()),
            None => Err(RuntimeError::Unavailable),
        };
        released?;
        guard.custody_released = true;
        Ok(guard)
    }
    /// Admission and preflight share one reservation, including before spawn.
    fn reserve_upgrade_choice(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, RuntimeError> {
        self.choice
            .clone()
            .try_lock_owned()
            .map_err(|_| RuntimeError::Busy)
    }
    /// Denial-only preflight; full mutation authority remains in WorkspaceUpgrade.
    fn validate_upgrade_transaction(
        &self,
        data: &std::path::Path,
        local: &std::path::Path,
        transaction: bridge_runtime_protocol::Uuid,
    ) -> Result<crate::workspace_upgrade::UpgradeStatus, RuntimeError> {
        self.upgrade_idle()?;
        crate::workspace_upgrade::validate_transaction(data, local, transaction)
    }
    fn upgrade_idle(&self) -> Result<(), RuntimeError> {
        if self.upgrading.load(std::sync::atomic::Ordering::Acquire)
            || (matches!(
                self.status(),
                LocalVmStatus::Preparing | LocalVmStatus::Starting
            ) && self
                .upgrade_cache
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|dto| dto.state == "fresh"))
        {
            Err(RuntimeError::Busy)
        } else {
            Ok(())
        }
    }
    /// Recovery never starts A in this app, even after an explicit restore.
    pub async fn restore_workspace(
        &self,
        data: std::path::PathBuf,
        local: std::path::PathBuf,
        transaction: bridge_runtime_protocol::Uuid,
    ) -> Result<crate::workspace_upgrade::UpgradeStatusDto, RuntimeError> {
        let choice = self.reserve_upgrade_choice()?;
        if self.validate_upgrade_transaction(&data, &local, transaction)?
            == crate::workspace_upgrade::UpgradeStatus::Restored
        {
            return Err(RuntimeError::RetainedState);
        }
        let _quiesced = self.quiesce_with_choice(choice).await?;
        let result = (|| {
            let mut owner = crate::workspace_upgrade::WorkspaceUpgrade::open(&data, &local)?;
            owner.restore(transaction)?;
            self.retire_workspace_state();
            self.block_for_recovery();
            Ok(owner.status_dto(native::upgrade_source().as_ref()))
        })();
        self.finish_upgrade_status(&data, &local, result)
    }
    /// Explicit association consent only; restart validated B, never Connect.
    pub async fn transfer_workspace_credential(
        self: &Arc<Self>,
        data: std::path::PathBuf,
        local: std::path::PathBuf,
        transaction: bridge_runtime_protocol::Uuid,
    ) -> Result<crate::workspace_upgrade::UpgradeStatusDto, RuntimeError> {
        let choice = self.reserve_upgrade_choice()?;
        if self.validate_upgrade_transaction(&data, &local, transaction)?
            != crate::workspace_upgrade::UpgradeStatus::Consumed
        {
            return Err(RuntimeError::RetainedState);
        }
        crate::workspace_upgrade::WorkspaceUpgrade::open(&data, &local)?
            .validate_transfer(transaction)?;
        let quiesced = self.quiesce_with_choice(choice).await?;
        let result = (|| {
            let mut owner = crate::workspace_upgrade::WorkspaceUpgrade::open(&data, &local)?;
            owner.transfer_credential(transaction)?;
            Ok(owner.status_dto(native::upgrade_source().as_ref()))
        })();
        let dto = self.finish_upgrade_status(&data, &local, result)?;
        drop(quiesced);
        if matches!(dto.state, "fresh" | "consumed") {
            self.start();
        }
        Ok(dto)
    }
    /// The slot is always native-derived, never a renderer argument.
    pub async fn remove_workspace_credential(
        self: &Arc<Self>,
        data: std::path::PathBuf,
        local: std::path::PathBuf,
        transaction: bridge_runtime_protocol::Uuid,
    ) -> Result<crate::workspace_upgrade::UpgradeStatusDto, RuntimeError> {
        let choice = self.reserve_upgrade_choice()?;
        if !matches!(
            self.validate_upgrade_transaction(&data, &local, transaction)?,
            crate::workspace_upgrade::UpgradeStatus::Consumed
                | crate::workspace_upgrade::UpgradeStatus::Restored
        ) {
            return Err(RuntimeError::RetainedState);
        }
        let slot = crate::workspace_upgrade::WorkspaceUpgrade::open(&data, &local)?
            .resolve_removal_slot(transaction)?;
        let quiesced = self.quiesce_with_choice(choice).await?;
        let result = (|| {
            let mut owner = crate::workspace_upgrade::WorkspaceUpgrade::open(&data, &local)?;
            owner.remove_credential(
                transaction,
                slot,
                &crate::runtime::credential_store::OsCredentialStore,
            )?;
            Ok(owner.status_dto(native::upgrade_source().as_ref()))
        })();
        let dto = self.finish_upgrade_status(&data, &local, result)?;
        drop(quiesced);
        if matches!(dto.state, "fresh" | "consumed") {
            self.start();
        }
        Ok(dto)
    }
    /// Installed once by the native shell; never a renderer-supplied callback.
    pub fn on_workspace_replaced(&self, observer: Arc<dyn Fn() + Send + Sync>) {
        *self.workspace_replaced_observer.lock().unwrap() = Some(observer);
    }
    fn retire_workspace_state(&self) {
        let observer = self.workspace_replaced_observer.lock().unwrap().clone();
        if let Some(observer) = observer {
            observer();
        }
    }
    /// Observe the durable transaction before ordinary startup takes its journal.
    pub fn seed_upgrade_status(&self, data: &std::path::Path, local: &std::path::Path) {
        let dto = crate::workspace_upgrade::upgrade_status(
            data,
            local,
            native::upgrade_source().as_ref(),
        );
        self.set_workspace_replaced(dto.transaction.is_some());
        self.cache_upgrade_status(dto);
    }
    fn quarantine_upgrade(&self) {
        let mut dto = self
            .upgrade_cache
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(crate::workspace_upgrade::UpgradeStatusDto::none);
        dto.state = "conflict";
        dto.error = Some("stopUnconfirmed");
        dto.busy = false;
        dto.can_replace = false;
        dto.can_restore = false;
        self.cache_upgrade_status(dto);
        self.inner.fail(
            "Workspace shutdown or custody release could not be confirmed.".into(),
            &RuntimeError::StopUnconfirmed("Workspace recovery is required.".into()),
            WorkspaceOperation::Stop,
            false,
        );
        // Keep upgrading=true: only process restart may release this quarantine.
    }
    fn cache_upgrade_status(&self, dto: crate::workspace_upgrade::UpgradeStatusDto) {
        *self.upgrade_cache.lock().unwrap() = Some(dto);
    }
    fn finish_upgrade_status(
        &self,
        data: &std::path::Path,
        local: &std::path::Path,
        result: Result<crate::workspace_upgrade::UpgradeStatusDto, RuntimeError>,
    ) -> Result<crate::workspace_upgrade::UpgradeStatusDto, RuntimeError> {
        match result {
            Ok(dto) => {
                self.cache_upgrade_status(dto.clone());
                Ok(dto)
            }
            Err(error) => {
                self.inner.fail(
                    failure_reason(&error),
                    &error,
                    WorkspaceOperation::Prepare,
                    false,
                );
                let mut dto = crate::workspace_upgrade::upgrade_status(
                    data,
                    local,
                    native::upgrade_source().as_ref(),
                );
                if dto.busy {
                    dto = self
                        .upgrade_cache
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or_else(crate::workspace_upgrade::UpgradeStatusDto::none);
                }
                dto.busy = false;
                dto.error = crate::workspace_upgrade::classify_error(&error);
                self.cache_upgrade_status(dto);
                Err(error)
            }
        }
    }
    /// Projection is not mutation authority. Our running journal is expected
    /// to exclude Journal::open; it is not evidence of an upgrade in progress.
    pub async fn upgrade_status(
        &self,
        data: std::path::PathBuf,
        local: std::path::PathBuf,
    ) -> crate::workspace_upgrade::UpgradeStatusDto {
        if self.upgrading.load(std::sync::atomic::Ordering::Acquire) {
            let mut dto = self
                .upgrade_cache
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(crate::workspace_upgrade::UpgradeStatusDto::none);
            dto.busy = dto.error != Some("stopUnconfirmed");
            return dto;
        }
        let phase = self.status();
        if matches!(
            phase,
            LocalVmStatus::Preparing | LocalVmStatus::Starting | LocalVmStatus::Ready
        ) {
            let mut dto = self
                .upgrade_cache
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(crate::workspace_upgrade::UpgradeStatusDto::none);
            dto.busy = dto.state == "fresh" && phase != LocalVmStatus::Ready;
            if phase == LocalVmStatus::Ready && dto.state == "fresh" {
                dto.state = "consumed";
                self.cache_upgrade_status(dto.clone());
            }
            return dto;
        }
        let source = native::upgrade_source();
        match tokio::task::spawn_blocking(move || {
            crate::workspace_upgrade::upgrade_status(&data, &local, source.as_ref())
        })
        .await
        {
            Ok(mut dto) => {
                // A failed native launch can still retain our journal.
                if dto.busy {
                    dto = self.upgrade_cache.lock().unwrap().clone().unwrap_or(dto);
                }
                dto.busy = self.upgrading.load(std::sync::atomic::Ordering::Acquire);
                if let Some(error) = self
                    .upgrade_cache
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|c| c.error)
                {
                    dto.error = Some(error);
                }
                self.cache_upgrade_status(dto.clone());
                dto
            }
            Err(_) => {
                let mut dto = self
                    .upgrade_cache
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(crate::workspace_upgrade::UpgradeStatusDto::none);
                dto.busy = false;
                self.cache_upgrade_status(dto.clone());
                dto
            }
        }
    }
    /// The owned task holds quiescence through the blocking worker AND handoff.
    /// Dropping an IPC waiter cannot detach filesystem work from lifecycle custody.
    pub async fn replace_workspace(
        self: &Arc<Self>,
        data: std::path::PathBuf,
        local: std::path::PathBuf,
        target: UpgradeTarget,
    ) -> Result<crate::workspace_upgrade::UpgradeStatusDto, RuntimeError> {
        let choice = self.reserve_upgrade_choice()?;
        self.upgrade_idle()?;
        if self.status() == LocalVmStatus::Ready {
            return Err(RuntimeError::RetainedState);
        }
        crate::workspace_upgrade::validate_new_replacement(&data, &local)?;
        let vm = self.clone();
        tokio::spawn(async move {
            let factory = vm.factory.clone().ok_or(RuntimeError::Unavailable)?;
            let quiesced = vm.quiesce_with_choice(choice).await?;
            vm.set_workspace_replaced(true);
            let (tx, rx) = watch::channel(false);
            *vm.upgrade_cancel.lock().unwrap() = Some(tx);
            let runtime = tokio::runtime::Handle::current();
            let inner = vm.inner.clone();
            inner.publish(LocalProgress::Status(LocalVmStatus::Preparing));
            let (worker_data, worker_local) = (data.clone(), local.clone());
            let result = tokio::task::spawn_blocking(move || {
                let mut owner =
                    crate::workspace_upgrade::WorkspaceUpgrade::open(&worker_data, &worker_local)?;
                owner.set_progress(Arc::new(move |event| {
                    inner.publish(LocalProgress::Image(event))
                }));
                owner.replace(&target.source, target.machine_id, &rx, &runtime)?;
                // Status must inspect the original creator Journal before handoff.
                let dto = owner.status_dto(Some(&target.source));
                let journal = owner
                    .take_destination()
                    .ok_or(RuntimeError::RetainedState)?;
                factory.adopt_destination(journal)?;
                Ok(dto)
            })
            .await
            .unwrap_or(Err(RuntimeError::Unavailable));
            *vm.upgrade_cancel.lock().unwrap() = None;
            let dto = vm.finish_upgrade_status(&data, &local, result)?;
            vm.retire_workspace_state();
            vm.recovery_blocked
                .store(false, std::sync::atomic::Ordering::Release);
            vm.inner
                .publish(LocalProgress::Status(LocalVmStatus::Cancelled));
            drop(quiesced);
            vm.start();
            Ok(dto)
        })
        .await
        .map_err(|_| RuntimeError::Unavailable)?
    }
    /// Retry creates a new destination, never reopens new-only authority.
    pub async fn retry_workspace(
        self: &Arc<Self>,
        data: std::path::PathBuf,
        local: std::path::PathBuf,
        target: UpgradeTarget,
        transaction: bridge_runtime_protocol::Uuid,
    ) -> Result<crate::workspace_upgrade::UpgradeStatusDto, RuntimeError> {
        let choice = self.reserve_upgrade_choice()?;
        if !matches!(
            self.validate_upgrade_transaction(&data, &local, transaction)?,
            crate::workspace_upgrade::UpgradeStatus::SourceSelected
                | crate::workspace_upgrade::UpgradeStatus::SourceRetained
                | crate::workspace_upgrade::UpgradeStatus::Incomplete
        ) {
            return Err(RuntimeError::RetainedState);
        }
        let vm = self.clone();
        tokio::spawn(async move {
            let factory = vm.factory.clone().ok_or(RuntimeError::Unavailable)?;
            let quiesced = vm.quiesce_with_choice(choice).await?;
            vm.set_workspace_replaced(true);
            let (tx, rx) = watch::channel(false);
            *vm.upgrade_cancel.lock().unwrap() = Some(tx);
            let runtime = tokio::runtime::Handle::current();
            let inner = vm.inner.clone();
            inner.publish(LocalProgress::Status(LocalVmStatus::Preparing));
            let (worker_data, worker_local) = (data.clone(), local.clone());
            let result = tokio::task::spawn_blocking(move || {
                let mut owner =
                    crate::workspace_upgrade::WorkspaceUpgrade::open(&worker_data, &worker_local)?;
                owner.set_progress(Arc::new(move |event| {
                    inner.publish(LocalProgress::Image(event))
                }));
                owner.retry(
                    transaction,
                    &target.source,
                    target.machine_id,
                    &rx,
                    &runtime,
                )?;
                let dto = owner.status_dto(Some(&target.source));
                let journal = owner
                    .take_destination()
                    .ok_or(RuntimeError::RetainedState)?;
                factory.adopt_destination(journal)?;
                Ok(dto)
            })
            .await
            .unwrap_or(Err(RuntimeError::Unavailable));
            *vm.upgrade_cancel.lock().unwrap() = None;
            let dto = vm.finish_upgrade_status(&data, &local, result)?;
            vm.retire_workspace_state();
            vm.recovery_blocked
                .store(false, std::sync::atomic::Ordering::Release);
            vm.inner
                .publish(LocalProgress::Status(LocalVmStatus::Cancelled));
            drop(quiesced);
            vm.start();
            Ok(dto)
        })
        .await
        .map_err(|_| RuntimeError::Unavailable)?
    }
    /// Cooperative cancellation at the existing acquisition/publication checkpoints.
    pub async fn cancel_workspace_upgrade(&self) -> Result<(), RuntimeError> {
        let cancelling_transaction = {
            let cancel = self.upgrade_cancel.lock().unwrap();
            if let Some(tx) = cancel.as_ref() {
                let _ = tx.send(true);
                true
            } else {
                false
            }
        };
        // Stop startup cooperatively without dropping the factory's original
        // new-only journal: Cancel is not quiesce/release_stopped.
        if !cancelling_transaction {
            self.cancel().await?;
        }
        Ok(())
    }
    /// Atomic Run/Chat admission, separate from each bounded VM request.
    pub fn try_authoring(&self) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.authoring.lock().unwrap().clone().try_lock_owned().ok()
    }
    /// Reject a captured operation whose admission belongs to an earlier boot.
    pub fn authoring_current(&self, lease: &tokio::sync::OwnedMutexGuard<()>) -> bool {
        Arc::ptr_eq(
            &self.authoring.lock().unwrap(),
            tokio::sync::OwnedMutexGuard::mutex(lease),
        )
    }
    pub fn status(&self) -> LocalVmStatus {
        self.inner.state.lock().unwrap().phase
    }
    /// Native-set only; latches true and never clears within this process.
    /// Main sets this once from startup recovery; later replace/retry/restore
    /// completions in this same process must call it again on success.
    pub fn set_workspace_replaced(&self, replaced: bool) {
        if replaced {
            self.workspace_replaced
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }
    pub fn workspace_replaced(&self) -> bool {
        self.workspace_replaced
            .load(std::sync::atomic::Ordering::Acquire)
    }
    /// Acknowledgment retained by the native session, independent of renderer reloads.
    pub fn model_connected(&self) -> bool {
        let state = self.inner.state.lock().unwrap();
        !state.closing && state.phase == LocalVmStatus::Ready && state.model_connected
    }
    pub fn run_cancel_supported(&self) -> bool {
        let state = self.inner.state.lock().unwrap();
        !state.closing && state.phase == LocalVmStatus::Ready && state.run_cancel_supported
    }
    /// A successful guest observation can invalidate an earlier Connect acknowledgment.
    pub fn clear_model_acknowledgment(&self) {
        self.inner.state.lock().unwrap().model_connected = false;
    }
    fn snapshot(&self, mut snapshot: crate::setup::SetupSnapshot) -> crate::setup::SetupSnapshot {
        let state = self.inner.state.lock().unwrap();
        snapshot.workspace_progress = state.progress.as_ref().map(ProgressClock::snapshot);
        snapshot.preflight_failure = if state.phase == LocalVmStatus::Failed {
            state.preflight_failure
        } else {
            None
        };
        snapshot.workspace_failure = if state.phase == LocalVmStatus::Failed {
            state.workspace_failure.clone()
        } else {
            None
        };
        // Publish phase, failure text and model availability from the same observation.
        snapshot
            .with_model_support(MODEL_IMAGE_AVAILABLE)
            .with_local_vm(state.phase.as_str(), state.failure.clone())
            .with_model(
                if state.phase == LocalVmStatus::Ready && MODEL_IMAGE_AVAILABLE {
                    "available"
                } else {
                    "locked"
                },
            )
    }
    pub fn start(self: &Arc<Self>) {
        let mut state = self.inner.state.lock().unwrap();
        if state.terminal
            || self
                .recovery_blocked
                .load(std::sync::atomic::Ordering::Acquire)
            || self.upgrading.load(std::sync::atomic::Ordering::Acquire)
            || state.closing
            || !matches!(
                state.phase,
                LocalVmStatus::Idle | LocalVmStatus::Cancelled | LocalVmStatus::Failed
            )
        {
            return;
        }
        let Some(factory) = self.factory.clone() else {
            return;
        };
        // Unknown work on an old boot must not own a new workspace's admission.
        *self.authoring.lock().unwrap() = Arc::new(AsyncMutex::new(()));
        state.phase = LocalVmStatus::Preparing;
        state.model_connected = false;
        state.run_cancel_supported = false;
        state.failure = None;
        state.preflight_failure = None;
        state.workspace_failure = None;
        state.operation = WorkspaceOperation::Verify;
        state.close_started = false;
        state.stopped = None;
        state.progress = None;
        state.cancelled = watch::channel(false).0;
        state.completion = watch::channel(None).0;
        let cancelled = state.cancelled.subscribe();
        let inner = self.inner.clone();
        // Stored before releasing the admission lock. Dropping an IPC waiter does
        // not drop this task or any installation/handshake job it awaits.
        state.startup = Some(tokio::spawn(async move {
            let progress = {
                let i = inner.clone();
                Arc::new(move |phase| i.publish(phase))
            };
            let result = factory.launch(cancelled.clone(), progress).await;
            let cancelled_error = *cancelled.borrow()
                && matches!(
                    &result,
                    Err(RuntimeError::Closed
                        | RuntimeError::Image(crate::runtime::image_types::ImageError::Cancelled))
                );
            // Native preflight/version policy refuses before creating a session.
            // Its retained journal still needs explicit release for replacement.
            let successful = result.is_ok()
                || cancelled_error
                || matches!(&result, Err(RuntimeError::Preflight(_)
                    | RuntimeError::Image(crate::runtime::image_types::ImageError::WorkspaceUpdateRequired
                        | crate::runtime::image_types::ImageError::WorkspaceVersionUnsupported)));
            let mut session = inner.session.lock().await;
            match result {
                Ok(live) => {
                    inner.state.lock().unwrap().run_cancel_supported = live.run_cancel_supported();
                    *session = Some(live);
                    inner.publish(LocalProgress::Status(LocalVmStatus::Ready));
                }
                Err(
                    RuntimeError::Closed
                    | RuntimeError::Image(crate::runtime::image_types::ImageError::Cancelled),
                ) if cancelled_error => {}
                Err(error) => {
                    let operation = inner.state.lock().unwrap().operation;
                    inner.fail(
                        format!("Starting the local VM failed: {}", failure_reason(&error)),
                        &error,
                        operation,
                        matches!(error, RuntimeError::StopUnconfirmed(_)),
                    )
                }
            }
            successful
        }));
    }
    /// Native callers keep typed results local; renderer commands wrap fixed work only.
    /// Closing during admitted work returns Closed after completion; effects may have applied.
    pub async fn with_client<T: Send + 'static>(
        &self,
        work: impl FnOnce(bridge_runtime_protocol::Uuid, &mut VmClient) -> T + Send + 'static,
    ) -> Result<T, RuntimeError> {
        if !self.inner.ready() {
            return Err(RuntimeError::Unavailable);
        }
        let mut session = self
            .inner
            .session
            .clone()
            .try_lock_owned()
            .map_err(|_| RuntimeError::Busy)?;
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            if !inner.ready() {
                return Err(RuntimeError::Closed);
            }
            let mut work = Some(work);
            let mut result = None;
            session
                .as_mut()
                .ok_or(RuntimeError::Unavailable)?
                .with_client(&mut |id, client| {
                    let work = work.take().ok_or(RuntimeError::Unavailable)?;
                    result = Some(work(id, client));
                    Ok(())
                })?;
            if !inner.ready() {
                return Err(RuntimeError::Closed);
            }
            result.ok_or(RuntimeError::Unavailable)
        })
        .await
        .map_err(|_| RuntimeError::Unavailable)?
    }
    pub async fn workspace_catalog(
        &self,
        setup: Arc<crate::setup::SetupStore>,
    ) -> Result<crate::catalog::CatalogReply, crate::catalog::CatalogError> {
        use crate::catalog::CatalogError;
        if !crate::catalog::IMAGE_AVAILABLE {
            return Err(CatalogError::CatalogUnavailable);
        }
        let failure = CatalogError::CatalogUnavailable;
        if !self.model_connected() {
            return Err(failure);
        }
        let result = self
            .with_client(move |id, client| setup.workspace_catalog(id, client))
            .await
            .map_err(CatalogError::from)?
            .map_err(|_| CatalogError::CatalogUnavailable)?;
        if !self.model_connected() {
            return Err(failure);
        }
        Ok(result)
    }
    pub async fn connect_model(
        &self,
        setup: Arc<crate::setup::SetupStore>,
    ) -> Result<crate::setup::ModelSetupState, crate::setup::SetupError> {
        self.connect_model_choice(setup, None).await
    }
    pub async fn connect_saved_key_model(
        &self,
        setup: Arc<crate::setup::SetupStore>,
        vendor: bridge_runtime_protocol::ModelVendor,
    ) -> Result<crate::setup::ModelSetupState, crate::setup::SetupError> {
        self.connect_model_choice(setup, Some(vendor)).await
    }
    async fn connect_model_choice(
        &self,
        setup: Arc<crate::setup::SetupStore>,
        vendor: Option<bridge_runtime_protocol::ModelVendor>,
    ) -> Result<crate::setup::ModelSetupState, crate::setup::SetupError> {
        if !MODEL_IMAGE_AVAILABLE {
            return Err(crate::setup::SetupError::model("modelGuest"));
        }
        let runtime = tokio::runtime::Handle::current();
        let inner = self.inner.clone();
        self.with_client(move |id, client| {
            let result =
                if let Some(vendor) = vendor {
                    setup.connect_saved_key_model(
                        id,
                        vendor,
                        client,
                        &crate::runtime::credential_store::OsCredentialStore,
                    )
                } else {
                    setup.connect_stored_model(
                    id, client, &crate::runtime::credential_store::OsCredentialStore,
                    || runtime.block_on(crate::runtime::credentials::capture_stored_github_token()),
                )
                };
            // The owned worker retains details even if its IPC waiter is dropped.
            inner.model_result(&result);
            result
        })
        .await
        .map_err(|error| {
            crate::setup::SetupError::model(if matches!(error, RuntimeError::Busy) {
                "modelBusy"
            } else {
                "modelSession"
            })
        })?
    }
    pub async fn connect_key_model(
        &self,
        setup: Arc<crate::setup::SetupStore>,
        vendor: bridge_runtime_protocol::ModelVendor,
        key: String,
    ) -> Result<crate::setup::ModelSetupState, crate::setup::SetupError> {
        if !MODEL_IMAGE_AVAILABLE {
            return Err(crate::setup::SetupError::model("modelGuest"));
        }
        let inner = self.inner.clone();
        self.with_client(move |id, client| {
            let result = setup.connect_key_model(
                id,
                vendor,
                &key,
                client,
                &crate::runtime::credential_store::OsCredentialStore,
            );
            // As with Copilot, the owned worker outlives a dropped IPC waiter.
            inner.model_result(&result);
            result
        })
        .await
        .map_err(|error| {
            crate::setup::SetupError::model(if matches!(error, RuntimeError::Busy) {
                "modelBusy"
            } else {
                "modelSession"
            })
        })?
    }
    /// Setup-only preparation shares VM custody with Connect/Remove, never general reconnect.
    pub async fn prepare_example(
        &self,
        setup: Arc<crate::setup::SetupStore>,
        folder: Arc<crate::agent_folder::AgentFolder>,
    ) -> Result<(String, &'static str), &'static str> {
        if !self.model_connected() {
            return Err("Connect the AI before preparing the example.");
        }
        self.with_client(move |_, _| {
            let binding = setup
                .model_binding_status()
                .map_err(|_| "The AI connection is unavailable.")?;
            if binding.unknown {
                return Err("The AI connection is unavailable.");
            }
            let connection = binding
                .connection
                .ok_or("The AI connection is unavailable.")?;
            let pair = crate::agent_folder::example_for_model(&connection.selection.model)
                .ok_or("The example model is unavailable.")?;
            folder.prepare_example_model(&connection.selection.model)?;
            Ok(pair)
        })
        .await
        .map_err(|error| {
            if matches!(error, RuntimeError::Busy) {
                "exampleBusy"
            } else {
                "The local cluster is unavailable."
            }
        })?
    }
    pub async fn reset_model(
        &self,
        setup: Arc<crate::setup::SetupStore>,
    ) -> Result<crate::setup::ModelSetupState, crate::setup::SetupError> {
        let inner = self.inner.clone();
        self.with_client(move |id, _client| {
            let result = setup
                .reset_mismatched_model(id, &crate::runtime::credential_store::OsCredentialStore);
            // A refused reset must not invalidate a newer, current-install connection.
            inner.model_reset_result(&result);
            result
        })
        .await
        .map_err(|error| {
            crate::setup::SetupError::model(if matches!(error, RuntimeError::Busy) {
                "modelBusy"
            } else {
                "modelSession"
            })
        })?
    }
    pub async fn remove_model(
        &self,
        setup: Arc<crate::setup::SetupStore>,
    ) -> Result<crate::setup::ModelSetupState, crate::setup::SetupError> {
        let inner = self.inner.clone();
        self.with_client(move |id, client| {
            let result = setup.sign_out_model(
                id,
                Some(client),
                &crate::runtime::credential_store::OsCredentialStore,
            );
            // Even a failed remove may already have stopped the guest model.
            inner.model_result(&result);
            result
        })
        .await
        .map_err(|error| {
            crate::setup::SetupError::model(if matches!(error, RuntimeError::Busy) {
                "modelBusy"
            } else {
                "modelSession"
            })
        })?
    }
    pub fn request_close(&self) {
        let mut state = self.inner.state.lock().unwrap();
        state.terminal = true;
        if state.stopped == Some(true) {
            state.phase = LocalVmStatus::Closed;
        } else if state.stopped.is_none() {
            state.phase = LocalVmStatus::Closing;
        }
        state.closing = true;
        state.progress = None;
        state.cancelled.send_replace(true);
    }
    pub async fn cancel(&self) -> Result<(), RuntimeError> {
        self.stop(false).await
    }
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        self.request_close();
        self.stop(true).await
    }
    async fn stop(&self, terminal: bool) -> Result<(), RuntimeError> {
        let mut completion = {
            let mut state = self.inner.state.lock().unwrap();
            state.terminal |= terminal;
            if !state.close_started {
                state.closing = true;
                state.phase = LocalVmStatus::Closing;
                state.progress = None;
                state.cancelled.send_replace(true);
                state.close_started = true;
                let startup = state.startup.take();
                let inner = self.inner.clone();
                // Custody survives dropped cancel, change and shutdown IPC waiters.
                tokio::spawn(async move {
                    let started = match startup {
                        Some(task) => matches!(task.await, Ok(true)),
                        None => true,
                    };
                    let mut session = inner.session.clone().lock_owned().await;
                    let stop_inner = inner.clone();
                    let stopped = tokio::task::spawn_blocking(move || {
                        match session.as_mut().map_or(Ok(()), |s| s.stop()) {
                            Ok(()) => {
                                session.take();
                                true
                            }
                            Err(error) => {
                                // Never replace a late session whose native stop failed.
                                stop_inner.fail(
                                    format!(
                                        "Stopping the local VM failed: {}",
                                        failure_reason(&error)
                                    ),
                                    &error,
                                    WorkspaceOperation::Stop,
                                    true,
                                );
                                false
                            }
                        }
                    })
                    .await
                    .unwrap_or(false)
                        && started;
                    let mut state = inner.state.lock().unwrap();
                    state.phase = if !stopped {
                        LocalVmStatus::Failed
                    } else if state.terminal {
                        LocalVmStatus::Closed
                    } else {
                        LocalVmStatus::Cancelled
                    };
                    state.stopped = Some(stopped);
                    state.closing = state.terminal || !stopped;
                    state.completion.send_replace(Some(stopped));
                });
            }
            state.completion.subscribe()
        };
        loop {
            if let Some(ok) = *completion.borrow_and_update() {
                return if ok {
                    Ok(())
                } else {
                    Err(RuntimeError::Unavailable)
                };
            }
            completion
                .changed()
                .await
                .map_err(|_| RuntimeError::Unavailable)?;
        }
    }
}
impl EngineTransport for LocalVm {
    fn execute(&self, request: EngineRequest) -> TransportFuture<'_> {
        Box::pin(async move {
            if !self.inner.ready() {
                return Err(EngineError::new(ErrorKind::Unavailable));
            }
            let mut session = self
                .inner
                .session
                .clone()
                .try_lock_owned()
                .map_err(|_| EngineError::new(ErrorKind::Execution))?;
            let inner = self.inner.clone();
            let runtime = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                if !inner.ready() {
                    return Err(EngineError::new(ErrorKind::Unavailable));
                }
                let session = session
                    .as_mut()
                    .ok_or_else(|| EngineError::new(ErrorKind::Unavailable))?;
                // Keep the owned session guard through Mac's existing inner job,
                // even if the renderer cancels its outer request.
                let result = runtime.block_on(session.execute(request));
                if inner.ready() {
                    result
                } else {
                    Err(EngineError::new(ErrorKind::Unavailable))
                }
            })
            .await
            .map_err(|_| EngineError::new(ErrorKind::Execution))?
        })
    }
}
