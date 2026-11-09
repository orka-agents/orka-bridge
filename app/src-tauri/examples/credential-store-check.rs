//! Dummy-only native store check. Never reads an existing sign-in or accepts a token.
use bridge_engine::runtime::credential_store::{CredentialStore, OsCredentialStore};
use bridge_runtime_protocol::Uuid;

fn main() -> std::process::ExitCode {
    let slot = Uuid::new_v4();
    let store = OsCredentialStore;
    let result = (|| {
        store.save(slot, b"bridge-synthetic-only")?;
        println!("save: OK");
        let loaded = store.load(slot)?;
        if loaded.as_bytes() != b"bridge-synthetic-only" {
            eprintln!("load: dummy payload mismatch");
            return Err(bridge_engine::runtime::credential_store::StoreError::Invalid);
        }
        println!("load: OK");
        Ok(())
    })();
    if let Err(error) = &result {
        eprintln!("round trip: {error}");
    }
    // Even a failed save can have applied. Delete only this run's fresh slot.
    let cleanup = store.delete(slot);
    match &cleanup {
        Ok(()) => println!("delete: OK"),
        Err(error) => eprintln!("delete: {error}"),
    }
    if result.is_ok() && cleanup.is_ok() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
