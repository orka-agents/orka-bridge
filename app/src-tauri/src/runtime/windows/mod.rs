//! Windows launch inputs and native capabilities. No Setup or renderer entry point.
#[cfg(windows)]
pub mod control;
pub mod inputs;
pub mod job;
pub mod launch;
#[cfg(windows)]
pub mod security;
