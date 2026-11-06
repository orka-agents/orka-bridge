//! Windows launch inputs and native capabilities. No Setup or renderer entry point.
#[cfg(windows)]
pub mod control;
pub mod inputs;
pub mod job;
pub mod launch;
pub mod qmp;
#[cfg(windows)]
pub mod qmp_io;
#[cfg(windows)]
pub mod security;
