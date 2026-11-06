use serde_json::Value;
use std::io;

pub const WINDOW_MS: u64 = 30_000;
pub struct GuardProbe {
    pub event: String,
    pub deadline_tick: u64,
}
impl GuardProbe {
    pub fn parse(event: &str, deadline: &str, now: u64) -> io::Result<Self> {
        let suffix = event
            .strip_prefix("Local\\bridge-guard-")
            .ok_or(io::ErrorKind::InvalidInput)?;
        if suffix.len() != 32
            || !suffix
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let probe = Self {
            event: event.into(),
            deadline_tick: deadline.parse().map_err(io::Error::other)?,
        };
        probe.remaining(now)?;
        Ok(probe)
    }
    pub fn remaining(&self, now: u64) -> io::Result<u32> {
        let remaining = self
            .deadline_tick
            .checked_sub(now)
            .filter(|&n| n > 0 && n <= WINDOW_MS)
            .ok_or(io::ErrorKind::TimedOut)?;
        Ok(remaining as u32)
    }
}
pub fn direct_guard_property(value: &Value) -> bool {
    [
        "knownGuardClosed",
        "explicitGuardInheritance",
        "busyWhileChildAlive",
        "ownedChildStopObserved",
        "freeWhileParentAlive",
    ]
    .iter()
    .all(|field| value[*field] == true)
}
pub fn missing_pss_type(phase: &str, index: usize, handle: usize, flags: i32) -> io::Error {
    io::Error::other(format!("phase={phase} entry={index} handle={handle:#x} flags={flags:#x}: snapshot lacks object type name"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_pss_entry_reports_phase_index_handle_and_flags() {
        let error = missing_pss_type("guard-parent-before-close", 17, 0x48, 0);
        assert_eq!(error.kind(), io::ErrorKind::Other);
        let message = error.to_string();
        for field in [
            "phase=guard-parent-before-close",
            "entry=17",
            "handle=0x48",
            "flags=0x0",
            "snapshot lacks object type name",
        ] {
            assert!(message.contains(field), "missing {field}: {message}");
        }
    }
    #[test]
    fn direct_guard_proof_does_not_depend_on_parent_inventory() {
        let proof = serde_json::json!({"knownGuardClosed":true,"explicitGuardInheritance":true,
            "busyWhileChildAlive":true,"ownedChildStopObserved":true,
            "freeWhileParentAlive":true,"parentPssError":"missing type"});
        assert!(direct_guard_property(&proof));
        for field in [
            "knownGuardClosed",
            "explicitGuardInheritance",
            "busyWhileChildAlive",
            "ownedChildStopObserved",
            "freeWhileParentAlive",
        ] {
            let mut failed = proof.clone();
            failed[field] = Value::Bool(false);
            assert!(!direct_guard_property(&failed), "{field}");
        }
        assert!(!direct_guard_property(&serde_json::json!({})));
    }
    fn name() -> String {
        format!("Local\\bridge-guard-{}", "a".repeat(32))
    }
    #[test]
    fn guard_arguments_reject_namespace_replays_and_unbounded_deadlines() {
        assert!(GuardProbe::parse(&name(), "31000", 1000).is_ok());
        for bad in [
            "Global\\bridge-guard-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "Local\\bridge-guard-abc",
            "Local\\bridge-guard-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\0",
        ] {
            assert!(GuardProbe::parse(bad, "31000", 1000).is_err(), "{bad:?}");
        }
        for tick in ["1000", "999", "31001", "18446744073709551615", "-1", "wat"] {
            assert!(GuardProbe::parse(&name(), tick, 1000).is_err(), "{tick}");
        }
    }
    #[test]
    fn guard_probe_uses_one_absolute_deadline() {
        let probe = GuardProbe::parse(&name(), "31000", 1000).unwrap();
        assert_eq!(probe.remaining(2000).unwrap(), 29000);
        assert_eq!(probe.remaining(30999).unwrap(), 1);
        assert!(probe.remaining(31000).is_err());
        assert!(probe.remaining(31001).is_err());
    }
}
