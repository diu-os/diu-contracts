#![cfg_attr(not(any(test, feature = "export-abi")), no_main)]
#![allow(unexpected_cfgs)]
extern crate alloc;

/// Shared pause state module — embedded in contracts that need emergency stop (ADR D-029).
pub mod pause;

/// DIURegistry — user identity, ORCID linking, verification.
/// Default entrypoint for WASM builds.
#[cfg(any(
    test,
    feature = "registry",
    not(any(
        feature = "reputation",
        feature = "achievements",
        feature = "token",
        feature = "progress"
    ))
))]
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

/// DIUProgress — simulation runs, module completions, XP cross-contract awards.
/// Use `--features progress` to build as WASM entrypoint.
#[cfg(any(test, feature = "progress"))]
pub mod progress;
