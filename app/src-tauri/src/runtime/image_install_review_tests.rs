use super::*;

#[tokio::test]
async fn activation_retries_and_reopen_require_the_namespace_flush_to_finish() {
    let mut outcomes = Vec::new();
    for committed in [false, true] {
        let scene = Scene::new();
        let mut target = scene.target();
        let (_sender, closed) = watch::channel(false);
        if committed {
            scene.install().await;
        } else {
            target.point = Some(Arc::new(|p| {
                if p == "generation-published" {
                    Err(ImageError::Storage(std::io::ErrorKind::Interrupted))
                } else {
                    Ok(())
                }
            }));
            materialize_image(&scene.image, &target, closed.clone(), |_| {})
                .await
                .unwrap_err();
        }
        target.point = Some(Arc::new(|p| {
            if p == "namespace-flush" {
                Err(ImageError::Storage(std::io::ErrorKind::Interrupted))
            } else {
                Ok(())
            }
        }));
        outcomes.push(
            materialize_image(&scene.image, &target, closed, |_| {})
                .await
                .err(),
        );
        if committed {
            outcomes.push(open_installed_image(&target).err());
        }
    }
    assert_eq!(
        outcomes,
        vec![Some(ImageError::Storage(std::io::ErrorKind::Interrupted)); 3],
        "no usable capability may escape a failed pending namespace flush"
    );
}
#[tokio::test]
async fn cancellation_observed_after_the_commit_does_not_report_a_failed_install() {
    let scene = Scene::new();
    let mut target = scene.target();
    let (sender, closed) = watch::channel(false);
    target.point = Some(Arc::new(move |p| {
        if p == "binding-published" {
            sender.send(true).unwrap();
        }
        Ok(())
    }));
    materialize_image(&scene.image, &target, closed, |_| {})
        .await
        .expect("the durable commit is the cancellation linearization point");
    open_installed_image(&scene.target()).unwrap();
}
#[tokio::test]
async fn incompatible_acquisition_cannot_create_private_payload_work() {
    for platform in [false, true] {
        let scene = Scene::new();
        let mut target = scene.target();
        if platform {
            target.target = ImageTarget::MacosArm64;
        } else {
            target.compatibility = "different-state";
        }
        let (_sender, closed) = watch::channel(false);
        let error = materialize_image(&scene.image, &target, closed, |_| {})
            .await
            .err();
        assert_eq!(
            error,
            Some(if platform {
                ImageError::Incompatible(Incompatibility::Target)
            } else {
                ImageError::RetainedImage
            })
        );
        assert!(!scene.path().join("image").exists());
    }
}
#[tokio::test]
async fn detached_active_materialization_refuses_an_overlapping_retry_with_busy() {
    let scene = Scene::new();
    let mut target = scene.target();
    let (arrived, mut arrived_rx) = tokio::sync::oneshot::channel();
    let arrived = std::sync::Mutex::new(Some(arrived));
    let (release, wait) = std::sync::mpsc::channel();
    let wait = std::sync::Mutex::new(wait);
    target.point = Some(Arc::new(move |p| {
        if p == "copy-chunk" {
            if let Some(tx) = arrived.lock().unwrap().take() {
                let _ = tx.send(());
                wait.lock().unwrap().recv().unwrap();
            }
        }
        Ok(())
    }));
    let (_sender, closed) = watch::channel(false);
    let mut first = Box::pin(materialize_image(
        &scene.image,
        &target,
        closed.clone(),
        |_| {},
    ));
    tokio::select! {result=&mut first=>panic!("first job returned before the gate: {result:?}"),result=&mut arrived_rx=>result.unwrap()}
    drop(first);
    let result = materialize_image(&scene.image, &scene.target(), closed, |_| {}).await;
    release.send(()).unwrap();
    assert_eq!(result.err(), Some(ImageError::Busy));
    let contender = regular(&scene.root, "image-owner.lock", true, false).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if contender.try_lock().is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await
        }
    })
    .await
    .unwrap();
    assert!(!scene.path().join("image-binding.json").exists());
}
