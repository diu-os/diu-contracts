#![cfg_attr(not(any(test, feature = "export-abi")), no_main)]
#![allow(unexpected_cfgs)]
extern crate alloc;

/// DIURegistry — user identity, ORCID linking, verification.
/// Default entrypoint for WASM builds.
#[cfg(any(test, not(feature = "reputation")))]
pub mod registry;

/// DIUReputation — XP tracking, levels, daily login streaks.
/// Use `--features reputation` to build as WASM entrypoint.
#[cfg(any(test, feature = "reputation"))]
pub mod reputation;
