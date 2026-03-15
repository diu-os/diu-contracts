//! PauseStorage — shared emergency stop module for DIU OS contracts (ADR D-029).
//!
//! Not a contract — no `#[entrypoint]`. Embed `PauseStorage pause_state;`
//! at the **END** of any contract's `sol_storage!` struct (D-025 storage discipline).
//!
//! # Usage
//! 1. Add `PauseStorage pause_state;` at the end of your `sol_storage!` struct.
//! 2. In `initialize()`: `self.pause_state.set_pause_guardian(owner);`
//! 3. In mutating functions: `self.pause_state.require_not_paused()?;`
//! 4. Expose `pause()`/`unpause()` in your contract's `#[public]` impl.
//!
//! Two pause flows are supported:
//! - **Admin-based** (`DIUToken`): contract does `require_admin()` then `internal_pause()`.
//! - **Guardian-based** (future contracts): contract calls `pause(caller)` directly.
//!
//! See ADR D-029 in ARCHITECTURE.md for full design rationale.

use stylus_sdk::{alloy_primitives::Address, alloy_sol_types::sol, prelude::*};

// ═══════════════════════════════════════════════════════════════════════════
// EVENTS
// ═══════════════════════════════════════════════════════════════════════════

sol! {
    /// Emitted when a contract is paused.
    event Paused(address indexed guardian);

    /// Emitted when a contract is unpaused.
    event Unpaused(address indexed guardian);
}

// ═══════════════════════════════════════════════════════════════════════════
// ERRORS
// ═══════════════════════════════════════════════════════════════════════════

sol! {
    error ContractPaused();
    error NotPauseGuardian();
    error ContractNotPaused();
}

/// Pause-specific error type used by [`PauseStorage`] methods.
#[derive(SolidityError)]
pub enum PauseError {
    /// Mutable operation attempted while the contract is paused.
    ContractPaused(ContractPaused),
    /// Caller is not the pause guardian.
    NotPauseGuardian(NotPauseGuardian),
    /// Unpause attempted when the contract is not paused.
    ContractNotPaused(ContractNotPaused),
}

impl core::fmt::Debug for PauseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ContractPaused(_) => write!(f, "ContractPaused"),
            Self::NotPauseGuardian(_) => write!(f, "NotPauseGuardian"),
            Self::ContractNotPaused(_) => write!(f, "ContractNotPaused"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STORAGE
// ═══════════════════════════════════════════════════════════════════════════

sol_storage! {
    /// Shared pause state. Embed at the END of any contract's storage struct (ADR D-025).
    ///
    /// Slot layout (relative to embedding position):
    ///   +0  paused          (bool)
    ///   +1  pause_guardian  (address)
    pub struct PauseStorage {
        /// Whether the contract is paused.
        bool paused;
        /// Address authorised to pause/unpause without timelock (ADR D-027/D-029).
        address pause_guardian;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// METHODS
// ═══════════════════════════════════════════════════════════════════════════

impl PauseStorage {
    // ─── Guards ──────────────────────────────────────────────────────

    /// Returns an error if the contract is paused.
    pub fn require_not_paused(&self) -> Result<(), PauseError> {
        if self.paused.get() {
            return Err(PauseError::ContractPaused(ContractPaused {}));
        }
        Ok(())
    }

    /// Returns an error if `caller` is not the pause guardian.
    pub fn require_pause_guardian(&self, caller: Address) -> Result<(), PauseError> {
        if caller != self.pause_guardian.get() {
            return Err(PauseError::NotPauseGuardian(NotPauseGuardian {}));
        }
        Ok(())
    }

    // ─── Guardian-based state mutations ──────────────────────────────

    /// Pause the contract. Caller must be the pause guardian.
    ///
    /// For future contracts using pure guardian-based pause (ADR D-029).
    /// `DIUToken` uses [`internal_pause`] instead (admin-based, backwards-compat).
    pub fn pause(&mut self, caller: Address) -> Result<(), PauseError> {
        self.require_pause_guardian(caller)?;
        self.require_not_paused()?;
        self.paused.set(true);
        Ok(())
    }

    /// Unpause the contract. Caller must be the pause guardian.
    pub fn unpause(&mut self, caller: Address) -> Result<(), PauseError> {
        self.require_pause_guardian(caller)?;
        if !self.paused.get() {
            return Err(PauseError::ContractNotPaused(ContractNotPaused {}));
        }
        self.paused.set(false);
        Ok(())
    }

    // ─── Internal state mutations (no access control) ─────────────────

    /// Pause without access control. Caller must enforce its own access check first.
    ///
    /// Used by `DIUToken.pause()` which calls `require_admin()` before this.
    pub fn internal_pause(&mut self) -> Result<(), PauseError> {
        self.require_not_paused()?;
        self.paused.set(true);
        Ok(())
    }

    /// Unpause without access control. Caller must enforce its own access check first.
    pub fn internal_unpause(&mut self) -> Result<(), PauseError> {
        if !self.paused.get() {
            return Err(PauseError::ContractNotPaused(ContractNotPaused {}));
        }
        self.paused.set(false);
        Ok(())
    }

    // ─── Guardian management ─────────────────────────────────────────

    /// Set the pause guardian address. No access control — callers must enforce their own.
    pub fn set_pause_guardian(&mut self, new_guardian: Address) {
        self.pause_guardian.set(new_guardian);
    }

    // ─── Views ───────────────────────────────────────────────────────

    /// Returns `true` if the contract is paused.
    pub fn is_paused(&self) -> bool {
        self.paused.get()
    }

    /// Returns the current pause guardian address.
    pub fn get_pause_guardian(&self) -> Address {
        self.pause_guardian.get()
    }
}
