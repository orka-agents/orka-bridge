//! Permanent, privacy-safe failure context for bridge-guest.
//!
//! Every helper here accepts only compile-time `&'static str` stage/phase
//! labels plus an `io::ErrorKind` and a raw OS errno. It never formats an
//! `io::Error`'s `Display`/`Debug` output, its source, request data, command
//! output, descriptors or credentials, and it never changes control flow:
//! callers get back the exact `io::Result` they passed in, unchanged.
use std::io;

/// A fixed, caller-chosen location for a failure: which stage (early/serve/
/// cli) and which static-code-labeled phase within it.
#[derive(Clone, Copy)]
pub struct Context {
    pub stage: &'static str,
    pub phase: &'static str,
}
impl Context {
    pub const fn new(stage: &'static str, phase: &'static str) -> Self {
        Self { stage, phase }
    }
}

fn line(ctx: Context, kind: io::ErrorKind, errno: Option<i32>) -> String {
    reason_line(ctx, None, kind, errno)
}
fn reason_line(
    ctx: Context,
    reason: Option<&'static str>,
    kind: io::ErrorKind,
    errno: Option<i32>,
) -> String {
    let suffix = reason.map(|r| format!(".{r}")).unwrap_or_default();
    let errno = errno
        .map(|n| n.to_string())
        .unwrap_or_else(|| "none".into());
    format!(
        "bridge-guest: stage={} phase={}{suffix} kind={kind:?} errno={errno}",
        ctx.stage, ctx.phase
    )
}

/// Logs a failure from an already-known kind/errno pair (no `io::Error` to
/// hand, e.g. a synthetic timeout or a self-invalid guard).
///
/// Best-effort: a broken or full stderr must never panic or turn a
/// wire-reported/readiness error into a process crash.
pub fn log(ctx: Context, kind: io::ErrorKind, errno: Option<i32>) {
    use std::io::Write as _;
    let _ = writeln!(io::stderr().lock(), "{}", line(ctx, kind, errno));
}

/// Keeps a shared I/O helper's fixed reason attached to its caller's phase.
pub fn log_reason(ctx: Context, reason: &'static str, kind: io::ErrorKind, errno: Option<i32>) {
    use std::io::Write as _;
    let _ = writeln!(
        io::stderr().lock(),
        "{}",
        reason_line(ctx, Some(reason), kind, errno)
    );
}
pub fn note_reason(ctx: Context, reason: &'static str, err: &io::Error) {
    log_reason(ctx, reason, err.kind(), err.raw_os_error());
}

/// Logs the kind/errno of a real `io::Error` without formatting anything
/// else about it (no Display, no source).
pub fn note(ctx: Context, err: &io::Error) {
    log(ctx, err.kind(), err.raw_os_error());
}

/// Logs `result`'s error (if any) and returns it completely unchanged.
pub fn context<T>(ctx: Context, result: io::Result<T>) -> io::Result<T> {
    if let Err(e) = &result {
        note(ctx, e);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_shape_with_errno() {
        let ctx = Context::new("early", "port-open");
        let got = line(ctx, io::ErrorKind::PermissionDenied, Some(13));
        assert_eq!(
            got,
            "bridge-guest: stage=early phase=port-open kind=PermissionDenied errno=13"
        );
    }

    #[test]
    fn exact_shape_without_errno() {
        let ctx = Context::new("serve", "wait");
        let got = line(ctx, io::ErrorKind::TimedOut, None);
        assert_eq!(
            got,
            "bridge-guest: stage=serve phase=wait kind=TimedOut errno=none"
        );
    }

    #[test]
    fn stage_and_phase_are_restricted_to_the_caller_literals() {
        for stage in ["early", "serve", "cli"] {
            for phase in ["probe", "boot-read", "frame-validation"] {
                let got = line(Context::new(stage, phase), io::ErrorKind::Other, None);
                assert!(got.starts_with(&format!("bridge-guest: stage={stage} phase={phase} ")));
            }
        }
    }

    #[test]
    fn secret_custom_error_content_never_appears() {
        let secret = "sk-super-secret-token-value";
        let err = io::Error::other(secret);
        let ctx = Context::new("early", "initialize");
        let got = line(ctx, err.kind(), err.raw_os_error());
        assert!(!got.contains(secret));
        assert!(!got.contains(&err.to_string()));
        assert_eq!(
            got,
            "bridge-guest: stage=early phase=initialize kind=Other errno=none"
        );
    }

    #[test]
    fn context_preserves_the_original_error_by_kind_and_errno() {
        let original: io::Result<()> = Err(io::Error::from_raw_os_error(2));
        let ctx = Context::new("serve", "port-canonicalize");
        let round_tripped = context(ctx, original);
        let e = round_tripped.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        assert_eq!(e.raw_os_error(), Some(2));
    }

    #[test]
    fn context_preserves_a_custom_payload_by_downcast() {
        #[derive(Debug)]
        struct Marker(u32);
        impl std::fmt::Display for Marker {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "marker")
            }
        }
        impl std::error::Error for Marker {}
        let original: io::Result<()> = Err(io::Error::other(Marker(7)));
        let got = context(Context::new("cli", "invalid-input"), original).unwrap_err();
        let marker = got.into_inner().unwrap().downcast::<Marker>().unwrap();
        assert_eq!(marker.0, 7);
    }

    #[test]
    fn context_passes_ok_through_untouched() {
        let original: io::Result<u32> = Ok(42);
        assert_eq!(
            context(Context::new("early", "probe"), original).unwrap(),
            42
        );
    }
}
