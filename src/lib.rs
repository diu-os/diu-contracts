#![cfg_attr(not(any(test, feature = "export-abi")), no_main)]
#![allow(unexpected_cfgs)]
extern crate alloc;

/// DIURegistry — user identity, ORCID linking, verification.
/// Default entrypoint for WASM builds.
#[cfg(any(test, not(any(feature = "reputation", feature = "achievements", feature = "token"))))]
pub mod registry;

/// DIUReputation — XP tracking, levels, daily login streaks.
/// Use `--features reputation` to build as WASM entrypoint.
#[cfg(any(test, feature = "reputation"))]
pub mod reputation;

/// DIUAchievements — soulbound ERC-721 NFT badges and certificates.
/// Use `--features achievements` to build as WASM entrypoint.
#[cfg(any(test, feature = "achievements"))]
pub mod achievements;

/// DIUToken — ERC-20 platform token with restricted mint and pause.
/// Use `--features token` to build as WASM entrypoint.
#[cfg(any(test, feature = "token"))]
pub mod token;
