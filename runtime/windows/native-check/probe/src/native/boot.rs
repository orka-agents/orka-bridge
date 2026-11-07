use super::*;
use crate::runtime::{
    self,
    image_types::ImageTarget,
    types::{BackendKind, VmBackend},
};
use std::io::Seek;
fn runtime_pin(pin: &PathBuf) -> io::Result<Value> {
    let pin: Value = serde_json::from_slice(&fs::read(pin)?).map_err(io::Error::other)?;
    check(pin["head"] == crate::SOURCE_HEAD && pin["producerRun"] == 42120608528u64)?;
    Ok(pin)
}
fn save(root: &std::path::Path, receipt: Value) -> io::Result<()> {
    fs::write(
        root.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).map_err(io::Error::other)?,
    )?;
    emit(receipt)
}
#[cfg(debug_assertions)]
pub fn run(args: &[String]) -> io::Result<()> {
    check(args.len() == 4)?;
    let runtime = PathBuf::from(&args[0]);
    let trusted = runtime_pin(&PathBuf::from(&args[1]))?;
    let factory = PathBuf::from(&args[2]);
    let root = PathBuf::from(&args[3]);
    // The selected folder is verified by the actual debug engineering-image
    // verifier before any Journal lock; no synthetic bytes enter this boot.
    std::env::set_var("BRIDGE_DEV_IMAGE_DIR", &factory);
    let image = crate::dev_image::from_environment()
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::other("image not selected"))?;
    let mut file = image.manifest().try_clone()?.into_std();
    file.seek(std::io::SeekFrom::Start(0))?;
    let receipt: Value = serde_json::from_reader(file).map_err(io::Error::other)?;
    let mut direct = serde_json::Map::new();
    for key in [
        "kernel",
        "initrd",
        "rootSelector",
        "kernelRelease",
        "commandLine",
        "requiredDrivers",
    ] {
        direct.insert(key.into(), receipt[key].clone());
    }
    let mut provenance = serde_json::Map::new();
    for key in [
        "diskSha256",
        "buildReceiptSha256",
        "inputSha256",
        "kernelConfigSha256",
        "modulesSha256",
        "endpointSha256",
        "earlyHookSha256",
        "buildHookSha256",
        "extractorSha256",
        "inspectorSha256",
    ] {
        provenance.insert(key.into(), receipt[key].clone());
    }
    provenance.insert("bootReceiptSha256".into(), image.declared(0).1.into());
    direct.insert("provenance".into(), provenance.into());
    let pin: ReviewedWindowsPin =
        serde_json::from_value(json!({"runtimeFiles":trusted["runtimeFiles"],
        "image":{"name":"os.qcow2","bytes":image.declared(3).0,"sha256":image.declared(3).1},
        "endpointSha256":receipt["endpointSha256"],"directBoot":direct}))
        .map_err(io::Error::other)?;
    let inputs = Arc::new(
        verify_windows_inputs(
            &pin,
            WindowsInputPaths {
                runtime,
                kernel: factory.join("os.kernel"),
                initrd: factory.join("os.initrd"),
                factory: factory.join("os.qcow2"),
            },
        )
        .map_err(io::Error::other)?,
    );
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (_sender, closed) = tokio::sync::watch::channel(false);
    let prepared = rt
        .block_on(runtime::setup_adapter::prepare_development_image(
            &image,
            closed.clone(),
        ))
        .map_err(|e| io::Error::other(format!("{e:?}")))?;
    let journal =
        runtime::journal::Journal::create(&root, BackendKind::Qemu, b"pc-q35-11.0".to_vec())
            .map_err(io::Error::other)?;
    let target = journal
        .image_target(crate::dev_image::STATE_COMPATIBILITY_ID)
        .map_err(io::Error::other)?;
    let installed = rt
        .block_on(prepared.materialize(&target, closed, |_| {}))
        .map_err(|e| io::Error::other(format!("{e:?}")))?;
    let backend = runtime::windows::backend::WindowsBackend::new(inputs);
    let failure = backend.failure_diagnostics();
    let worker = runtime::RuntimeWorker::new(Box::new(backend), journal, Arc::new(installed))
        .map_err(io::Error::other)?;
    let host_utc = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let started = std::time::Instant::now();
    let mut session = match runtime::windows::session::WindowsSession::start(worker, host_utc) {
        Ok(session) => session,
        Err(error) => {
            fs::write(root.join("qemu-stderr.txt"), failure.stderr()?)?;
            save(
                &root,
                json!({"status":"failed","sourceHead":crate::SOURCE_HEAD,"guestBoot":false,"error":error.to_string(),"elapsedMs":started.elapsed().as_millis()}),
            )?;
            return Err(io::Error::other(error));
        }
    };
    let ready = session.client_mut().map_err(io::Error::other)?.readiness();
    let shutdown = session.shutdown();
    let diagnostic = session.diagnostics().map_err(io::Error::other)?;
    fs::write(root.join("qmp-rx.jsonl"), diagnostic.rx)?;
    fs::write(root.join("qmp-tx.jsonl"), diagnostic.tx)?;
    fs::write(root.join("qemu-stderr.txt"), diagnostic.stderr)?;
    let passed = ready.is_ok() && shutdown.is_ok();
    drop(session);
    drop(image);
    drop(rt);
    let free = runtime::journal::Journal::open(&root).is_ok();
    let receipt = json!({"status":if passed&&free {"passed"} else {"failed"},"sourceHead":crate::SOURCE_HEAD,
        "producerRun":trusted["producerRun"],"runtimeZipSha256":trusted["runtimeZipSha256"],
        "guestBoot":true,"accelerator":"whpx","earlyAndServeReady":true,"clientReadiness":ready.is_ok(),
        "shutdownAckAndExit0AndWholeJobStop":shutdown.is_ok(),"ownershipReleased":free,
        "elapsedMs":started.elapsed().as_millis(),"modelExecution":false,"tokenExecution":false});
    save(&root, receipt)?;
    if passed && free {
        println!("PASS: Windows backend/session WHPX boot and observed shutdown");
        Ok(())
    } else {
        Err(io::Error::other("WHPX boot did not complete cleanly"))
    }
}
pub fn state(args: &[String]) -> io::Result<()> {
    check(args.len() == 3)?;
    let runtime = PathBuf::from(&args[0]);
    eprintln!("state stage: trusted runtime pin");
    let trusted = runtime_pin(&PathBuf::from(&args[1]))?;
    let work = PathBuf::from(&args[2]);
    fs::create_dir(&work)?;
    let (config, _) = super::controller::packet(&runtime, &PathBuf::from(&args[1]), &work)?;
    let cfg: Config = serde_json::from_slice(&fs::read(config)?).map_err(io::Error::other)?;
    eprintln!("state stage: verify runtime and inert inputs");
    let inputs = Arc::new(
        verify_windows_inputs(
            &cfg.pin,
            WindowsInputPaths {
                runtime,
                kernel: cfg.paths.kernel,
                initrd: cfg.paths.initrd,
                factory: cfg.paths.factory,
            },
        )
        .map_err(io::Error::other)?,
    );
    eprintln!("state stage: create held Journal");
    let root = work.join("installation");
    let journal =
        runtime::journal::Journal::create(&root, BackendKind::Qemu, b"pc-q35-11.0".to_vec())
            .map_err(io::Error::other)?;
    eprintln!("state stage: verify inert image source");
    let verified = crate::native_image_fixture::verified(&work, ImageTarget::WindowsAmd64);
    let target = journal
        .image_target(&verified.image().identity().state_compatibility_id)
        .map_err(io::Error::other)?;
    let (_sender, closed) = tokio::sync::watch::channel(false);
    let rt = tokio::runtime::Builder::new_current_thread().build()?;
    eprintln!("state stage: materialize inert installed image");
    let installed = rt
        .block_on(runtime::image_install::materialize_image(
            &verified,
            &target,
            closed,
            |_| {},
        ))
        .map_err(|e| io::Error::other(format!("{e:?}")))?;
    eprintln!("state stage: build borrowed-owner spec");
    let spec = runtime::probe_spec(&journal, Arc::new(installed));
    let before = runtime::identity::file_identity(&File::open(&spec.state_path)?)
        .map_err(io::Error::other)?;
    let mut backend = runtime::windows::backend::WindowsBackend::new(inputs);
    let failure = backend.failure_diagnostics();
    eprintln!("state stage: actual owned qemu-img create/copy");
    let result = backend.create_state(&spec);
    fs::write(work.join("qemu-img-stderr.txt"), failure.stderr()?)?;
    result.map_err(io::Error::other)?;
    check(
        before
            == runtime::identity::file_identity(&File::open(&spec.state_path)?)
                .map_err(io::Error::other)?,
    )?;
    check(File::open(&spec.state_path)?.metadata()?.len() > 0)?;
    save(
        &work,
        json!({"status":"passed","sourceHead":crate::SOURCE_HEAD,"producerRun":trusted["producerRun"],
        "actualQemuImg":true,"stateVirtualBytes":spec.state_bytes,"heldStateIdentityPreserved":true,
        "guestBoot":false,"whpxUsability":"not-tested"}),
    )?;
    println!("PASS: actual owned qemu-img state preparation");
    Ok(())
}
