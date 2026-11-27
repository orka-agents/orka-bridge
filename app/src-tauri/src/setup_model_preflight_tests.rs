//! The Copilot preflight runs before any binding is written as Pending and before any
//! sign-in is delivered, so its refusals leave nothing to clean up.
use super::*;
use crate::copilot_preflight::{Catalog, Preflight, PreflightFailure};

const TOKEN: &[u8] = b"gho_synthetic0preflight0only";
const LOGIN: &str = "synthetic-user";

fn setup_with(preflight: Preflight) -> (tempfile::TempDir, SetupStore, Uuid) {
    let tmp = tempfile::tempdir().unwrap();
    let setup = SetupStore::new(Some(tmp.path().to_owned())).with_copilot_preflight(preflight);
    setup.select_local().unwrap();
    (tmp, setup, Uuid::new_v4())
}
fn vault() -> FakeVault {
    FakeVault {
        secret: Some(TOKEN),
        ..FakeVault::default()
    }
}
fn capture() -> Result<CapturedGithubToken, crate::runtime::credentials::CaptureError> {
    CapturedGithubToken::try_from(TOKEN).map(|token| token.with_account(LOGIN))
}
fn refusals() -> Vec<(Preflight, &'static str)> {
    vec![
        (
            Preflight::fixed(Err(PreflightFailure::SignInRejected)),
            "modelCopilotSignInRejected",
        ),
        (
            Preflight::fixed(Err(PreflightFailure::AccessDenied)),
            "modelCopilotAccessDenied",
        ),
        (
            Preflight::fixed(Err(PreflightFailure::Unreachable)),
            "modelCopilotUnreachable",
        ),
        (
            Preflight::fixed(Err(PreflightFailure::Malformed)),
            "modelCopilotMalformed",
        ),
        (
            Preflight::fixed(Ok(Catalog::offering(&["other-model"]))),
            "modelCopilotModelNotOffered",
        ),
    ]
}
fn assert_no_token(error: &SetupError) {
    let encoded = serde_json::to_string(error).unwrap();
    let shown = format!("{error:?} {encoded} {:?}", error.model_detail());
    assert!(!shown.contains("synthetic0preflight"), "{shown}");
}

#[test]
fn first_connect_refused_by_the_preflight_saves_and_delivers_nothing() {
    for (preflight, kind) in refusals() {
        let (tmp, setup, install) = setup_with(preflight);
        let vault = vault();
        // Check again repeats the whole check; neither attempt leaves a record behind.
        for _ in 0..2 {
            let (mut client, worker) = peer(Uuid::new_v4(), |request| {
                assert!(
                    matches!(request.body, RequestBody::ModelStatus {}),
                    "a refused preflight must not reach credential or Connect requests"
                );
                Some(idle())
            });
            let error = setup
                .connect_stored_model(install, Some("gpt-5.6-terra"), &mut client, &vault, capture)
                .unwrap_err();
            drop(client);
            let traffic = worker.join().unwrap();
            assert_eq!(error.kind, kind);
            assert_eq!(error.account.as_deref(), Some(LOGIN));
            assert_eq!(traffic.publications(), 0);
            assert_eq!(traffic.frames.len(), 1, "only the idle probe");
            assert!(!tmp.path().join("setup-v1/model-v1").exists());
            let status = setup.model_binding_status().unwrap();
            assert!(!status.associated && !status.unknown);
            assert_no_token(&error);
            if kind == "modelCopilotModelNotOffered" {
                assert_eq!(error.wanted.as_deref(), Some("gpt-5.6-terra"));
                assert_eq!(
                    error.offered.as_deref(),
                    Some(&["other-model".to_string()][..])
                );
            }
        }
        assert_eq!(vault.counts(), (0, 0, 0));
    }
}

#[test]
fn classic_token_is_refused_before_saving() {
    let (tmp, setup, install) =
        setup_with(Preflight::fixed(Ok(Catalog::offering(&["gpt-5.6-terra"]))));
    let vault = vault();
    let (mut client, worker) = peer(Uuid::new_v4(), |_| Some(idle()));
    let error = setup
        .connect_stored_model(install, Some("gpt-5.6-terra"), &mut client, &vault, || {
            CapturedGithubToken::try_from(b"ghp_synthetic0classic".as_slice())
        })
        .unwrap_err();
    drop(client);
    assert_eq!(worker.join().unwrap().publications(), 0);
    assert_eq!(error.kind, "modelCopilotTokenType");
    assert!(!tmp.path().join("setup-v1/model-v1").exists());
    assert_eq!(vault.counts(), (0, 0, 0));
}

#[test]
fn saved_sign_in_refused_by_the_preflight_stays_stored_never_pending() {
    for (preflight, kind) in refusals() {
        let (_tmp, setup, install) = setup_with(preflight);
        let vault = vault();
        setup.associate_secret(install, TOKEN, &vault).unwrap();
        let root = setup.root.as_deref().unwrap().to_owned();
        let before = fs::read(root.join("model-v1")).unwrap();
        // A later boot as well as the same one: Stored never needs previous-boot cleanup.
        for _ in 0..2 {
            let (mut client, worker) = peer(Uuid::new_v4(), |request| {
                assert!(matches!(request.body, RequestBody::ModelStatus {}));
                Some(idle())
            });
            let error = setup
                .connect_stored_model(install, Some("gpt-5.6-terra"), &mut client, &vault, || {
                    panic!("a saved sign-in is never captured again")
                })
                .unwrap_err();
            drop(client);
            let traffic = worker.join().unwrap();
            assert_eq!(error.kind, kind);
            assert_ne!(error.kind, "modelReconnect");
            assert_eq!(traffic.publications(), 0);
            let b = read_binding(&root).unwrap().unwrap();
            assert_eq!(b.phase, Phase::Stored);
            assert!(b.nonce.is_none() && b.connection.is_none());
            assert_eq!(fs::read(root.join("model-v1")).unwrap(), before);
            assert!(!setup.model_binding_status().unwrap().unknown);
            assert_no_token(&error);
        }
        // Remove sign-in needs no workspace cleanup for a Stored record.
        setup.sign_out_model(install, None, &vault).unwrap();
        assert!(!root.join("model-v1").exists());
        assert_eq!(vault.counts(), (1, 2, 1));
    }
}

#[test]
fn the_chosen_model_is_what_the_guest_is_asked_to_connect() {
    let (_tmp, setup, install) = setup_with(Preflight::fixed(Ok(Catalog::offering(&[
        "gpt-4.1",
        "gpt-6.1-terra",
    ]))));
    let vault = vault();
    setup.associate_secret(install, TOKEN, &vault).unwrap();
    let generation = Uuid::new_v4();
    let chosen = ModelConnection {
        id: Uuid::new_v4(),
        generation,
        selection: ModelSelection {
            model: "gpt-6.1-terra".into(),
            ..selection()
        },
    };
    let expected = chosen.clone();
    let root = setup.root.as_deref().unwrap().to_owned();
    let (mut client, worker) = peer(Uuid::new_v4(), move |request| {
        Some(match &request.body {
            RequestBody::ModelStatus {} => idle(),
            RequestBody::CredentialStatus {} => {
                ResultBody::CredentialState(credential(None, false))
            }
            RequestBody::CredentialPublish { .. } => {
                ResultBody::CredentialState(credential(Some(generation), true))
            }
            RequestBody::ModelConnect { selection, .. } => {
                assert_eq!(selection.model, "gpt-6.1-terra");
                assert_eq!(read_binding(&root).unwrap().unwrap().phase, Phase::Pending);
                remote(&expected, ModelPhase::Ready)
            }
            _ => panic!("unexpected request"),
        })
    });
    let state = setup
        .connect_stored_model(install, Some("gpt-6.1-terra"), &mut client, &vault, || {
            panic!("no capture")
        })
        .unwrap();
    drop(client);
    assert_eq!(worker.join().unwrap().publications(), 1);
    assert!(state.ready);
    assert_eq!(state.connection.unwrap().selection.model, "gpt-6.1-terra");
}

#[test]
fn a_same_boot_retry_repeats_the_chosen_model() {
    let (_tmp, setup, install) =
        setup_with(Preflight::fixed(Ok(Catalog::offering(&["gpt-5.6-sol"]))));
    let vault = vault();
    setup.associate_secret(install, TOKEN, &vault).unwrap();
    let generation = Uuid::new_v4();
    let chosen = ModelConnection {
        id: Uuid::new_v4(),
        generation,
        selection: ModelSelection {
            model: "gpt-5.6-sol".into(),
            ..selection()
        },
    };
    let expected = chosen.clone();
    let mut connects = 0;
    let mut published = false;
    let (mut client, worker) = peer(Uuid::new_v4(), move |request| {
        Some(match &request.body {
            RequestBody::ModelStatus {} if !published => idle(),
            // After delivery the guest reports the sign-in installed but no connection.
            RequestBody::ModelStatus {} => ResultBody::ModelState(ModelState {
                retained_cleanup: true,
                diagnostic: None,
                phase: ModelPhase::Idle,
                connection: None,
                credential: credential(Some(generation), true),
            }),
            RequestBody::CredentialStatus {} => {
                ResultBody::CredentialState(credential(None, false))
            }
            RequestBody::CredentialPublish { .. } => {
                published = true;
                ResultBody::CredentialState(credential(Some(generation), true))
            }
            RequestBody::ModelConnect { selection, .. } => {
                connects += 1;
                assert_eq!(selection.model, "gpt-5.6-sol", "attempt {connects}");
                if connects == 1 {
                    ResultBody::ModelState(ModelState {
                        retained_cleanup: true,
                        diagnostic: None,
                        phase: ModelPhase::Idle,
                        connection: None,
                        credential: credential(Some(generation), true),
                    })
                } else {
                    remote(&expected, ModelPhase::Ready)
                }
            }
            _ => panic!("unexpected request"),
        })
    });
    let first =
        setup.connect_stored_model(install, Some("gpt-5.6-sol"), &mut client, &vault, || {
            panic!("no capture")
        });
    assert!(first.is_err());
    assert!(setup.model_binding_status().unwrap().unknown);
    let second = setup
        .connect_stored_model(install, Some("gpt-5.6-sol"), &mut client, &vault, || {
            panic!("no capture")
        })
        .unwrap();
    drop(client);
    assert_eq!(
        worker.join().unwrap().publications(),
        1,
        "never delivered twice"
    );
    assert_eq!(second.connection.unwrap().selection.model, "gpt-5.6-sol");
}

#[test]
fn checking_while_the_workspace_starts_writes_nothing() {
    let (tmp, setup, _) = setup_with(Preflight::fixed(Ok(Catalog::offering(&[
        "gpt-4.1",
        "gpt-6.1-terra",
    ]))));
    let vault = vault();
    let checked = setup.check_copilot(&vault, capture).unwrap();
    assert_eq!(
        serde_json::to_value(&checked).unwrap(),
        serde_json::json!({"account": LOGIN, "models": [{"id": "gpt-4.1"}, {"id": "gpt-6.1-terra"}], "preselected": "gpt-6.1-terra"})
    );
    for (preflight, kind) in refusals()
        .into_iter()
        .filter(|(_, kind)| *kind != "modelCopilotModelNotOffered")
    {
        let (tmp, setup, _) = setup_with(preflight);
        let error = setup.check_copilot(&vault, capture).unwrap_err();
        assert_eq!(error.kind, kind);
        assert!(!tmp.path().join("setup-v1/model-v1").exists());
        assert_no_token(&error);
    }
    // No usable model is an answer, not a failure: the card says so.
    let (_none, empty, _) = setup_with(Preflight::fixed(Ok(Catalog::offering(&[]))));
    let checked = empty.check_copilot(&vault, capture).unwrap();
    assert!(checked.models.is_empty() && checked.preselected.is_none());
    assert!(!tmp.path().join("setup-v1/model-v1").exists());
    assert_eq!(vault.counts(), (0, 0, 0));
    // A saved key is not a Copilot sign-in to check.
    let (_tmp, setup, install) = setup_with(Preflight::fixed(Ok(Catalog::offering(&["gpt-4.1"]))));
    let key = FakeVault {
        secret: Some(b"sk-synthetic"),
        ..FakeVault::default()
    };
    setup
        .associate_secret_for_vendor(
            install,
            b"sk-synthetic",
            Some(KeyVendor::Openai),
            None,
            &key,
        )
        .unwrap();
    assert_eq!(
        setup
            .check_copilot(&key, || panic!("no capture"))
            .unwrap_err()
            .kind,
        "modelAlreadyAssociated"
    );
}

#[test]
fn a_first_connect_without_a_chosen_model_reads_nothing() {
    let (tmp, setup, install) = setup_with(Preflight::fixed(Ok(Catalog::offering(&["gpt-4.1"]))));
    let vault = vault();
    let (mut client, worker) = peer(Uuid::new_v4(), |_| Some(idle()));
    let error = setup
        .connect_stored_model(install, None, &mut client, &vault, || {
            panic!("no sign-in is read before the person chooses a model")
        })
        .unwrap_err();
    drop(client);
    assert_eq!(worker.join().unwrap().publications(), 0);
    assert_eq!(error.kind, "modelCopilotChooseModel");
    assert!(!tmp.path().join("setup-v1/model-v1").exists());
    assert_eq!(vault.counts(), (0, 0, 0));
}
