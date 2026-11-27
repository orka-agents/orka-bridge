use super::*;
use crate::workspace_fresh::{fail_once, FreshStatusDto as Fresh, Refusal};

/// A launch that waits to be closed: the workspace is Ready only when a test says so.
struct Factory;
impl LocalFactory for Factory {
    fn launch(
        &self,
        mut closed: watch::Receiver<bool>,
        _: Arc<dyn Fn(LocalProgress) + Send + Sync>,
    ) -> StartFuture<'_> {
        Box::pin(async move {
            if !*closed.borrow() {
                let _ = closed.changed().await;
            }
            Err(RuntimeError::Closed)
        })
    }
}
struct Scene {
    _temp: tempfile::TempDir,
    data: std::path::PathBuf,
    local: std::path::PathBuf,
    store: Arc<crate::setup::SetupStore>,
    vm: Arc<LocalVm>,
}
fn scene() -> Scene {
    let temp = tempfile::tempdir().unwrap();
    let (data, local) = (temp.path().join("roaming"), temp.path().join("local"));
    std::fs::create_dir_all(data.join("setup-v1")).unwrap();
    std::fs::write(data.join("setup-v1/local-vm"), b"").unwrap();
    std::fs::create_dir_all(&local).unwrap();
    drop(
        crate::runtime::journal::Journal::create(
            &local.join("local-vm-v1"),
            crate::runtime::types::BackendKind::Apple,
            vec![1],
        )
        .unwrap(),
    );
    let store = Arc::new(crate::setup::SetupStore::with_upgrade_root(
        Some(data.clone()),
        Some(local.clone()),
    ));
    let vm = LocalVm::new(Arc::new(Factory));
    vm.set_fresh_root(data.clone(), local.clone());
    vm.inner
        .publish(LocalProgress::Status(LocalVmStatus::Ready));
    Scene {
        _temp: temp,
        data,
        local,
        store,
        vm,
    }
}
impl Scene {
    async fn start(&self) -> Fresh {
        self.vm
            .start_fresh_workspace(self.data.clone(), self.local.clone(), self.store.clone())
            .await
    }
    async fn status(&self) -> serde_json::Value {
        serde_json::to_value(setup_status(self.store.clone(), &self.vm).await.unwrap()).unwrap()
    }
}

#[tokio::test]
async fn a_failure_after_the_workspace_was_set_aside_is_reported_as_interrupted_never_unchanged() {
    let scene = scene();
    // Twice: the action's own retry meets the same failure.
    fail_once(&scene.local, "record retired");
    fail_once(&scene.local, "record retired");
    let Fresh::Interrupted { transaction } = scene.start().await else {
        panic!("a set-aside workspace must not be reported as unchanged");
    };
    assert!(!scene.local.join("local-vm-v1").exists());
    assert!(scene
        .local
        .join(format!("discarded-{transaction}"))
        .exists());
    // No workspace starts until Bridge finishes.
    scene.vm.start();
    assert_ne!(scene.vm.status(), LocalVmStatus::Preparing);
    let status = scene.status().await;
    assert_eq!(
        status["workspaceFresh"],
        serde_json::json!({"state": "interrupted", "transaction": transaction})
    );
    assert_eq!(status["localVm"], "failed");
    // Pressing it again finishes the same transaction and starts first install.
    let Fresh::Replaced {
        transaction: finished,
        ..
    } = scene.start().await
    else {
        panic!("the next try must finish it");
    };
    assert_eq!(finished, transaction);
    assert_eq!(scene.vm.status(), LocalVmStatus::Preparing);
    scene.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_refusal_after_the_stop_starts_the_workspace_again() {
    let scene = scene();
    std::fs::write(scene.local.join("local-vm-v1/record.json"), b"{}").unwrap();
    assert_eq!(
        scene.start().await,
        Fresh::Refused {
            reason: Refusal::Layout,
            restarted: true,
            needed_bytes: None,
            reclaimable: vec![],
        }
    );
    assert_eq!(scene.vm.status(), LocalVmStatus::Preparing);
    assert!(scene.local.join("local-vm-v1/record.json").exists());
    scene.vm.shutdown().await.unwrap();
}

#[tokio::test]
async fn busy_answers_never_replace_what_the_status_says_and_cleanup_results_are_kept() {
    let scene = scene();
    let left = scene.local.join("discarded-x").display().to_string();
    scene.vm.seed_fresh(Fresh::Replaced {
        transaction: bridge_runtime_protocol::Uuid::new_v4(),
        ai: crate::workspace_fresh::AiOutcome::Removed,
        reclaimable: vec![left.clone()],
        cleaning: false,
    });
    // Cleanup of an earlier fresh workspace holds the owner: busy before anything is stopped.
    let owner = crate::workspace_fresh::FreshWorkspace::open(&scene.data, &scene.local).unwrap();
    assert_eq!(scene.start().await, Fresh::refused(Refusal::Busy));
    assert_eq!(scene.vm.status(), LocalVmStatus::Ready);
    assert_eq!(scene.status().await["workspaceFresh"]["state"], "replaced");
    drop(owner);
    // Another refusal keeps reporting the folder that may still hold a sign-in copy.
    let updates = scene.local.join("workspace-upgrades-v1");
    std::fs::create_dir(&updates).unwrap();
    std::fs::write(updates.join("intent.json"), b"{}").unwrap();
    let reply = scene.start().await;
    assert_eq!(
        reply,
        Fresh::Refused {
            reason: Refusal::UpdateHistory,
            restarted: false,
            needed_bytes: None,
            reclaimable: vec![left.clone()],
        }
    );
    assert_eq!(scene.vm.status(), LocalVmStatus::Ready);
    // The status says why the action can't start, without taking any lock.
    let status = scene.status().await;
    assert_eq!(status["workspaceFreshPlan"]["blocked"], "updateHistory");
    assert_eq!(
        status["workspaceFresh"]["reclaimable"],
        serde_json::json!([left])
    );
}
