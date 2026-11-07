use super::*;

// Removing validation must fail before any native API can see these inputs.
#[test]
fn invalid_secrets_are_rejected_before_backend_access() {
    for secret in [
        b"".as_slice(),
        b"synthetic\n",
        b"synthetic secret",
        b"\x00",
        b"\xff",
    ] {
        assert_eq!(
            OsCredentialStore.save(Uuid::new_v4(), secret),
            Err(StoreError::Invalid)
        );
    }
}

#[test]
fn nil_slot_is_rejected_for_every_operation() {
    let store: &dyn CredentialStore = &OsCredentialStore;
    assert_eq!(
        store.save(Uuid::nil(), b"synthetic-only"),
        Err(StoreError::Invalid)
    );
    assert_eq!(store.load(Uuid::nil()), Err(StoreError::Invalid));
    assert_eq!(store.delete(Uuid::nil()), Err(StoreError::Invalid));
}

#[test]
fn oversized_save_is_rejected_without_truncation() {
    assert_eq!(
        OsCredentialStore.save(Uuid::new_v4(), &vec![b'x'; STORE_LIMIT + 1]),
        Err(StoreError::TooLarge)
    );
    assert_eq!(
        OsCredentialStore.save(Uuid::new_v4(), &vec![b'x'; MAX_CREDENTIAL + 1]),
        Err(StoreError::TooLarge)
    );
}

#[test]
fn loaded_payload_requires_bounded_printable_ascii() {
    for secret in [b"".as_slice(), b"synthetic\n", b"\xff"] {
        assert_eq!(payload(secret), Err(StoreError::Invalid));
    }
    assert_eq!(
        payload(&vec![b'x'; STORE_LIMIT + 1]),
        Err(StoreError::TooLarge)
    );
    let bytes = vec![b'x'; STORE_LIMIT];
    assert_eq!(payload(&bytes).unwrap().as_bytes(), bytes);
    assert_eq!(
        payload(b"synthetic-only").unwrap().as_bytes(),
        b"synthetic-only"
    );
}

#[test]
fn errors_have_only_static_redacted_text() {
    for (error, display, debug) in [
        (
            StoreError::Unavailable,
            "credential store unavailable",
            "Unavailable",
        ),
        (StoreError::Invalid, "invalid credential", "Invalid"),
        (StoreError::TooLarge, "credential too large", "TooLarge"),
    ] {
        assert_eq!(error.to_string(), display);
        assert_eq!(format!("{error:?}"), debug);
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
#[test]
fn unsupported_platform_has_no_storage_fallback() {
    let slot = Uuid::new_v4();
    assert_eq!(
        OsCredentialStore.save(slot, b"synthetic-only"),
        Err(StoreError::Unavailable)
    );
    assert_eq!(OsCredentialStore.load(slot), Err(StoreError::Unavailable));
    assert_eq!(OsCredentialStore.delete(slot), Err(StoreError::Unavailable));
}

#[test]
fn interaction_is_suppressed_and_previous_policy_restored_on_success_and_error() {
    use std::cell::{Cell, RefCell};
    for previous in [false, true] {
        for fail in [false, true] {
            let policy = Cell::new(previous);
            let calls = RefCell::new(Vec::new());
            let result = suppress_interaction(
                || {
                    calls.borrow_mut().push(("get", policy.get()));
                    Ok(policy.get())
                },
                |value| {
                    calls.borrow_mut().push(("set", value));
                    policy.set(value);
                    Ok(())
                },
                || {
                    calls.borrow_mut().push(("operation", policy.get()));
                    if fail {
                        Err(StoreError::Unavailable)
                    } else {
                        Ok(())
                    }
                },
            );
            assert_eq!(
                result,
                if fail {
                    Err(StoreError::Unavailable)
                } else {
                    Ok(())
                }
            );
            assert_eq!(policy.get(), previous);
            assert_eq!(
                *calls.borrow(),
                vec![
                    ("get", previous),
                    ("set", false),
                    ("operation", false),
                    ("set", previous)
                ]
            );
        }
    }
}

#[test]
fn interaction_setup_failure_does_not_execute_operation() {
    use std::cell::Cell;
    let ran = Cell::new(false);
    assert_eq!(
        suppress_interaction(
            || Err(StoreError::Unavailable),
            |_| Ok(()),
            || {
                ran.set(true);
                Ok(())
            }
        ),
        Err(StoreError::Unavailable)
    );
    assert!(!ran.get());
    assert_eq!(
        suppress_interaction(
            || Ok(true),
            |_| Err(StoreError::Unavailable),
            || {
                ran.set(true);
                Ok(())
            }
        ),
        Err(StoreError::Unavailable)
    );
    assert!(!ran.get());
}

#[test]
fn interaction_restore_failure_never_reports_native_effect_as_success() {
    use std::cell::Cell;
    let writes = Cell::new(0);
    let ran = Cell::new(false);
    let result = suppress_interaction(
        || Ok(true),
        |_| {
            writes.set(writes.get() + 1);
            if writes.get() == 2 {
                Err(StoreError::Unavailable)
            } else {
                Ok(())
            }
        },
        || {
            ran.set(true);
            Ok(())
        },
    );
    assert!(ran.get());
    assert_eq!(writes.get(), 2);
    assert_eq!(result, Err(StoreError::Unavailable));
}

#[test]
fn interaction_previous_policy_is_restored_before_resuming_unwind() {
    use std::cell::Cell;
    let policy = Cell::new(true);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), StoreError> = suppress_interaction(
            || Ok(policy.get()),
            |value| {
                policy.set(value);
                Ok(())
            },
            || {
                assert!(!policy.get());
                panic!("synthetic policy unwind")
            },
        );
    }));
    assert!(result.is_err());
    assert!(policy.get());
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
#[ignore = "requires explicit authorization for native credential-store I/O"]
fn native_synthetic_roundtrip_uses_only_a_new_owned_slot() {
    struct OwnedSlot(Uuid);
    impl Drop for OwnedSlot {
        fn drop(&mut self) {
            let _ = OsCredentialStore.delete(self.0);
        }
    }
    let owned = OwnedSlot(Uuid::new_v4());
    OsCredentialStore.save(owned.0, b"synthetic-only").unwrap();
    assert_eq!(
        OsCredentialStore.load(owned.0).unwrap().as_bytes(),
        b"synthetic-only"
    );
    OsCredentialStore.delete(owned.0).unwrap();
    OsCredentialStore.delete(owned.0).unwrap();
    assert_eq!(
        OsCredentialStore.load(owned.0),
        Err(StoreError::Unavailable)
    );
}
