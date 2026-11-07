//! Synthetic setup scenarios over the real established Unix control channel.
#![cfg(unix)]

use super::*;
use crate::runtime::credential_store::StoreError;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
    os::unix::net::UnixStream,
    thread::{self, JoinHandle},
    time::Duration,
};

const SECRET: &[u8] = b"synthetic-model-only";

#[derive(Default)]
struct FakeVault {
    deny_save: bool,
    deny_delete: Mutex<bool>,
    block_publish: Mutex<Option<std::path::PathBuf>>,
    calls: Mutex<Vec<(&'static str, Uuid)>>,
    slots: Mutex<HashMap<Uuid, Vec<u8>>>,
}
impl FakeVault {
    fn counts(&self) -> (usize, usize, usize) {
        let calls = self.calls.lock().unwrap();
        let count = |name| calls.iter().filter(|(op, _)| *op == name).count();
        (count("save"), count("load"), count("delete"))
    }
}
impl CredentialStore for FakeVault {
    fn save(&self, slot: Uuid, secret: &[u8]) -> Result<(), StoreError> {
        assert!(!slot.is_nil());
        assert_eq!(secret, SECRET);
        self.calls.lock().unwrap().push(("save", slot));
        if self.deny_save {
            return Err(StoreError::Unavailable);
        }
        self.slots.lock().unwrap().insert(slot, secret.to_vec());
        if let Some(path) = self.block_publish.lock().unwrap().take() {
            fs::create_dir(path).unwrap();
        }
        Ok(())
    }
    fn load(&self, slot: Uuid) -> Result<CredentialPayload, StoreError> {
        self.calls.lock().unwrap().push(("load", slot));
        let slots = self.slots.lock().unwrap();
        CredentialPayload::new(slots.get(&slot).ok_or(StoreError::Unavailable)?)
            .map_err(|_| StoreError::Invalid)
    }
    fn delete(&self, slot: Uuid) -> Result<(), StoreError> {
        self.calls.lock().unwrap().push(("delete", slot));
        if *self.deny_delete.lock().unwrap() {
            return Err(StoreError::Unavailable);
        }
        self.slots.lock().unwrap().remove(&slot);
        Ok(())
    }
}

struct Traffic {
    frames: Vec<WireRequest>,
    bytes: usize,
}
impl Traffic {
    fn publications(&self) -> usize {
        self.frames
            .iter()
            .filter(|r| matches!(r.body, RequestBody::CredentialPublish { .. }))
            .count()
    }
    fn assert_empty(&self) {
        assert_eq!(self.bytes, 0, "refusal must precede an actual wire write");
        assert!(self.frames.is_empty());
    }
}
// None means the peer received the request but lost its acknowledgement.
fn peer(
    nonce: Uuid,
    mut reply: impl FnMut(&WireRequest) -> Option<ResultBody> + Send + 'static,
) -> (VmClient, JoinHandle<Traffic>) {
    let (host, guest) = UnixStream::pair().unwrap();
    for stream in [&host, &guest] {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
    }
    let worker = thread::spawn(move || {
        let mut io = BufReader::new(guest);
        let mut traffic = Traffic {
            frames: Vec::new(),
            bytes: 0,
        };
        loop {
            let mut raw = Vec::new();
            if io.read_until(b'\n', &mut raw).unwrap() == 0 {
                break;
            }
            traffic.bytes += raw.len();
            let frame: Frame = serde_json::from_slice(&raw).unwrap();
            frame.validate().unwrap();
            let Frame::Request(request) = frame else {
                panic!("expected request")
            };
            let response = reply(&request);
            let id = request.request_id;
            let boot_nonce = request.boot_nonce;
            assert_eq!(boot_nonce, nonce);
            traffic.frames.push(request);
            let Some(body) = response else { break };
            let response = Frame::Result(WireResult {
                version: VERSION,
                boot_nonce,
                request_id: id,
                body,
            });
            response.validate().unwrap();
            io.get_mut().write_all(&encode(&response).unwrap()).unwrap();
        }
        traffic
    });
    (
        VmClient::established(Box::new(host), nonce).unwrap(),
        worker,
    )
}
fn selected_setup() -> (tempfile::TempDir, SetupStore, Uuid) {
    let tmp = tempfile::tempdir().unwrap();
    let setup = SetupStore::new(Some(tmp.path().to_owned()));
    setup.select_local().unwrap();
    (tmp, setup, Uuid::new_v4())
}
fn associated() -> (tempfile::TempDir, SetupStore, Uuid, FakeVault) {
    let (tmp, setup, install) = selected_setup();
    let vault = FakeVault::default();
    setup.associate_secret(install, SECRET, &vault).unwrap();
    (tmp, setup, install, vault)
}
fn selection() -> ModelSelection {
    ModelSelection {
        provider: "copilot".into(),
        model: "gpt-5.6-terra".into(),
    }
}
fn credential(generation: Option<Uuid>, installed: bool) -> CredentialState {
    CredentialState {
        profile: CREDENTIAL_PROFILE,
        generation,
        installed,
    }
}
fn idle() -> ResultBody {
    ResultBody::ModelState(ModelState {
        phase: ModelPhase::Idle,
        connection: None,
        credential: credential(None, false),
    })
}
fn connection() -> ModelConnection {
    ModelConnection {
        id: Uuid::new_v4(),
        selection: selection(),
        generation: Uuid::new_v4(),
    }
}
fn remote(connection: &ModelConnection, phase: ModelPhase) -> ResultBody {
    ResultBody::ModelState(ModelState {
        phase,
        connection: Some(connection.clone()),
        credential: credential(Some(connection.generation), true),
    })
}

#[test]
fn denied_save_leaves_no_association_plaintext_fallback_or_load() {
    let (_tmp, setup, install) = selected_setup();
    let vault = FakeVault {
        deny_save: true,
        ..FakeVault::default()
    };
    assert!(setup.associate_secret(install, SECRET, &vault).is_err());
    assert!(!setup.model_binding_status().unwrap().associated);
    let root = setup.root.as_deref().unwrap();
    assert!(read_binding(root).unwrap().is_none());
    let entries: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(entries, vec![root.join("local-vm")]);
    assert_eq!(fs::read(&entries[0]).unwrap(), b"");
    let (mut client, worker) = peer(Uuid::new_v4(), |_| panic!("no associated secret"));
    assert!(setup
        .connect_model(install, selection(), &mut client, &vault)
        .is_err());
    drop(client);
    worker.join().unwrap().assert_empty();
    assert_eq!(vault.counts(), (1, 0, 0));
    assert!(vault.slots.lock().unwrap().is_empty());
}

#[test]
fn wrong_installation_never_writes_to_guest_or_loads_owned_slot() {
    let (_tmp, setup, install, vault) = associated();
    let slot = read_binding(setup.root.as_deref().unwrap())
        .unwrap()
        .unwrap()
        .slot;
    let wrong = Uuid::new_v4();
    assert_ne!(wrong, install);
    let (mut client, worker) = peer(Uuid::new_v4(), |_| panic!("wrong installation"));
    assert!(setup
        .connect_model(wrong, selection(), &mut client, &vault)
        .is_err());
    assert!(setup
        .sign_out_model(wrong, Some(&mut client), &vault)
        .is_err());
    drop(client);
    worker.join().unwrap().assert_empty();
    assert_eq!(vault.counts(), (1, 0, 0));
    assert!(vault.slots.lock().unwrap().contains_key(&slot));
    assert_eq!(
        read_binding(setup.root.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .phase,
        Phase::Stored
    );
}

#[test]
fn older_peer_framed_unavailable_precedes_secret_load_and_delivery() {
    let (_tmp, setup, install, vault) = associated();
    let (mut client, worker) = peer(Uuid::new_v4(), |request| {
        assert!(matches!(request.body, RequestBody::ModelStatus {}));
        Some(ResultBody::Error {
            code: ErrorCode::Unavailable,
        })
    });
    assert!(setup
        .connect_model(install, selection(), &mut client, &vault)
        .is_err());
    drop(client);
    let traffic = worker.join().unwrap();
    assert_eq!(traffic.frames.len(), 1);
    assert!(traffic.bytes > 0);
    assert_eq!(traffic.publications(), 0);
    assert_eq!(vault.counts(), (1, 0, 0));
    assert_eq!(
        read_binding(setup.root.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .phase,
        Phase::Stored
    );
}

#[test]
fn lost_delivery_ack_is_one_publication_and_durable_pending_even_with_fresh_client() {
    let (_tmp, setup, install, vault) = associated();
    let root = setup.root.as_ref().unwrap().clone();
    let nonce = Uuid::new_v4();
    let (mut client, worker) = peer(nonce, move |request| match &request.body {
        RequestBody::ModelStatus {} => Some(idle()),
        RequestBody::CredentialStatus {} => {
            Some(ResultBody::CredentialState(credential(None, false)))
        }
        RequestBody::CredentialPublish {
            token,
            expected_generation,
        } => {
            assert_eq!(token.as_bytes(), SECRET);
            assert_eq!(*expected_generation, None);
            let binding = read_binding(&root).unwrap().unwrap();
            assert_eq!(binding.phase, Phase::Pending);
            assert_eq!(binding.nonce, Some(nonce));
            None
        }
        _ => panic!("no connect after lost publication acknowledgement"),
    });
    assert!(setup
        .connect_model(install, selection(), &mut client, &vault)
        .is_err());
    assert!(setup
        .connect_model(install, selection(), &mut client, &vault)
        .is_err());
    drop(client);
    let traffic = worker.join().unwrap();
    assert_eq!(traffic.frames.len(), 3);
    assert_eq!(traffic.publications(), 1);
    assert!(traffic.bytes > SECRET.len());
    let reopened = SetupStore::new(Some(_tmp.path().to_owned()));
    let (mut fresh, worker) = peer(Uuid::new_v4(), |_| panic!("pending cannot replay"));
    assert!(reopened
        .connect_model(install, selection(), &mut fresh, &vault)
        .is_err());
    drop(fresh);
    worker.join().unwrap().assert_empty();
    let binding = read_binding(setup.root.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(binding.phase, Phase::Pending);
    assert!(binding.connection.is_none());
    assert!(reopened.model_binding_status().unwrap().unknown);
    assert_eq!(vault.counts(), (1, 1, 0));
    assert_eq!(
        vault.slots.lock().unwrap().get(&binding.slot).unwrap(),
        SECRET
    );
}

#[test]
fn unknown_connect_retains_known_id_and_unknown_disconnect_never_clears_or_deletes() {
    let (_tmp, setup, install, vault) = associated();
    let conn = connection();
    let expected = conn.clone();
    let (mut client, worker) = peer(Uuid::new_v4(), move |request| {
        Some(match &request.body {
            RequestBody::ModelStatus {} => idle(),
            RequestBody::CredentialStatus {} => {
                ResultBody::CredentialState(credential(None, false))
            }
            RequestBody::CredentialPublish { token, .. } => {
                assert_eq!(token.as_bytes(), SECRET);
                ResultBody::CredentialState(credential(Some(expected.generation), true))
            }
            RequestBody::ModelConnect {
                generation,
                selection,
            } => {
                assert_eq!(*generation, expected.generation);
                assert_eq!(*selection, expected.selection);
                remote(&expected, ModelPhase::Unknown)
            }
            RequestBody::ModelDisconnect { connection } => {
                assert_eq!(*connection, expected.id);
                remote(&expected, ModelPhase::Unknown)
            }
            _ => panic!("unknown disconnect must not clear"),
        })
    });
    assert!(setup
        .connect_model(install, selection(), &mut client, &vault)
        .is_err());
    let root = setup.root.as_deref().unwrap();
    let binding = read_binding(root).unwrap().unwrap();
    assert_eq!(binding.phase, Phase::Pending);
    assert_eq!(binding.connection, Some(conn.clone()));
    assert!(setup
        .sign_out_model(install, Some(&mut client), &vault)
        .is_err());
    drop(client);
    let traffic = worker.join().unwrap();
    assert_eq!(traffic.frames.len(), 5);
    assert_eq!(traffic.publications(), 1);
    assert!(
        matches!(traffic.frames[4].body, RequestBody::ModelDisconnect { connection } if connection == conn.id)
    );
    assert_eq!(vault.counts(), (1, 1, 0));
    assert!(vault.slots.lock().unwrap().contains_key(&binding.slot));
    assert_eq!(read_binding(root).unwrap().unwrap().connection, Some(conn));
    assert!(setup.model_binding_status().unwrap().unknown);
}

#[test]
fn failed_same_lease_health_check_persists_unknown_without_load_or_republication() {
    let (_tmp, setup, install, vault) = associated();
    let root = setup.root.as_deref().unwrap();
    let nonce = Uuid::new_v4();
    let conn = connection();
    // Seed valid connected metadata: the existing core test covers initial Connect.
    let mut binding = read_binding(root).unwrap().unwrap();
    binding.phase = Phase::Connected;
    binding.nonce = Some(nonce);
    binding.connection = Some(conn.clone());
    write_binding(root, &binding).unwrap();
    let (mut client, worker) = peer(nonce, move |request| {
        assert!(matches!(request.body, RequestBody::ModelStatus {}));
        Some(remote(&conn, ModelPhase::Unknown))
    });
    assert!(setup
        .connect_model(install, selection(), &mut client, &vault)
        .is_err());
    drop(client);
    let traffic = worker.join().unwrap();
    assert_eq!(traffic.frames.len(), 1);
    assert_eq!(traffic.publications(), 0);
    assert_eq!(vault.counts(), (1, 0, 0));
    assert!(vault.slots.lock().unwrap().contains_key(&binding.slot));
    assert_eq!(read_binding(root).unwrap().unwrap().phase, Phase::Pending);
    assert!(setup.model_binding_status().unwrap().unknown);
    let (mut fresh, worker) = peer(nonce, |_| panic!("unknown health cannot retry Connect"));
    assert!(setup
        .connect_model(install, selection(), &mut fresh, &vault)
        .is_err());
    drop(fresh);
    worker.join().unwrap().assert_empty();
    assert_eq!(vault.counts(), (1, 0, 0));
}

#[test]
fn acknowledged_clear_allows_explicit_host_cleanup_retry_without_guest_or_load() {
    let (_tmp, setup, install, vault) = associated();
    let expected = connection();
    let conn = expected.clone();
    let mut published = false;
    let (mut client, worker) = peer(Uuid::new_v4(), move |request| {
        Some(match &request.body {
            RequestBody::ModelStatus {} => idle(),
            RequestBody::CredentialStatus {} => ResultBody::CredentialState(credential(
                if published {
                    Some(expected.generation)
                } else {
                    None
                },
                published,
            )),
            RequestBody::CredentialPublish { token, .. } => {
                assert_eq!(token.as_bytes(), SECRET);
                published = true;
                ResultBody::CredentialState(credential(Some(expected.generation), true))
            }
            RequestBody::ModelConnect { .. } => remote(&expected, ModelPhase::Ready),
            RequestBody::ModelDisconnect { connection } => {
                assert_eq!(*connection, expected.id);
                ResultBody::ModelState(ModelState {
                    phase: ModelPhase::Idle,
                    connection: None,
                    credential: credential(Some(expected.generation), true),
                })
            }
            RequestBody::CredentialClear { .. } => {
                ResultBody::CredentialState(credential(Some(Uuid::new_v4()), false))
            }
            _ => panic!("unexpected cleanup request"),
        })
    });
    setup
        .connect_model(install, conn.selection, &mut client, &vault)
        .unwrap();
    *vault.deny_delete.lock().unwrap() = true;
    assert!(setup
        .sign_out_model(install, Some(&mut client), &vault)
        .is_err());
    assert!(setup.model_binding_status().unwrap().associated);
    assert!(!setup.model_binding_status().unwrap().ready);
    *vault.deny_delete.lock().unwrap() = false;
    assert!(
        setup.sign_out_model(install, None, &vault).is_ok(),
        "acknowledged clear must permit exact host-only cleanup retry"
    );
    drop(client);
    let traffic = worker.join().unwrap();
    assert_eq!(traffic.frames.len(), 7);
    assert_eq!(traffic.publications(), 1);
    assert_eq!(vault.counts(), (1, 1, 2));
    assert!(!setup.model_binding_status().unwrap().associated);
}

#[test]
fn association_persist_failure_deletes_exact_new_slot_not_unowned_slot_or_file() {
    let (_tmp, setup, install) = selected_setup();
    let vault = FakeVault::default();
    let unowned = Uuid::new_v4();
    vault
        .slots
        .lock()
        .unwrap()
        .insert(unowned, b"unowned-synthetic".to_vec());
    let root = setup.root.as_deref().unwrap();
    let sentinel = b"unowned-file-unchanged";
    fs::write(root.join("unowned-note"), sentinel).unwrap();
    *vault.block_publish.lock().unwrap() = Some(root.join("model-v1"));
    assert!(setup.associate_secret(install, SECRET, &vault).is_err());
    {
        let calls = vault.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        let generated = calls[0].1;
        assert_ne!(generated, unowned);
        assert_eq!(*calls, vec![("save", generated), ("delete", generated)]);
        let slots = vault.slots.lock().unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots.get(&unowned).unwrap(), b"unowned-synthetic");
    }
    assert_eq!(fs::read(root.join("unowned-note")).unwrap(), sentinel);
    assert!(root.join("model-v1").is_dir());
    assert!(
        !fs::read_dir(root).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("model-next")),
        "failed publication must clean only its owned temporary"
    );
    fs::remove_dir(root.join("model-v1")).unwrap();
    assert!(
        setup
            .associate_secret(install, SECRET, &vault)
            .unwrap()
            .associated
    );
}

#[test]
fn unknown_crash_temporary_does_not_block_or_get_deleted_by_new_publication() {
    let (_tmp, setup, install) = selected_setup();
    let vault = FakeVault::default();
    let root = setup.root.as_deref().unwrap();
    let sentinel = b"unknown-crash-metadata";
    fs::write(root.join("model-next"), sentinel).unwrap();
    assert!(
        setup
            .associate_secret(install, SECRET, &vault)
            .unwrap()
            .associated
    );
    assert_eq!(fs::read(root.join("model-next")).unwrap(), sentinel);
}
