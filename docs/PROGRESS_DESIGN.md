# DIUProgress — Contract Design

**Status**: Design (pending approval) | **Phase**: 2
**Author**: Bakhtiyor Ruzimatov | **Date**: 27 Feb 2026
**ADRs**: D-019 (Simulation-First Research Loop), D-015 (3-Mode UX)

---

## Overview

DIUProgress tracks per-user simulation runs and learning module completions for
the DIU OS platform. It is the first contract with a cross-contract write: it
calls `DIUReputation.addXP` on every successful record to tie on-chain learning
events to the reputation system.

It also acts as the **Data Export Hook** mandated by ADR D-019: all simulation
events carry full parameters for backend indexing into JSON/CSV (Research Mode
export). An additional view function returns a complete on-chain snapshot for
direct export without requiring event indexing.

**Access Control** (from ARCHITECTURE.md access matrix):

| Caller | Allowed operations |
|--------|--------------------|
| Public | All view functions |
| Backend (authorized) | `record_simulation`, `record_module_completion` |
| Owner | `grant_authorized`, `revoke_authorized`, `set_reputation_contract` |
| Cross-contract | ← calls `DIUReputation.addXP` (outbound) |

---

## Storage Layout

```
sol_storage! {
    #[entrypoint]
    pub struct DIUProgress {
        // ── Admin ──────────────────────────────────────────────────────────
        /// Contract owner (set during initialize()).
        address owner;

        /// Whether the contract has been initialized.
        bool initialized;

        /// Authorized backend addresses that can record progress.
        mapping(address => bool) authorized;

        /// Address of the deployed DIUReputation contract.
        /// If Address::ZERO, cross-contract XP calls are skipped (test mode).
        address reputation_contract;

        // ── Bounds ─────────────────────────────────────────────────────────
        /// Maximum valid experiment_id (inclusive). Set at initialize().
        /// Example: 10 (experiments 0..=10 valid).
        uint256 max_experiment_id;

        /// Maximum valid module_id (inclusive). Set at initialize().
        /// Example: 5 (modules 0..=5 valid).
        uint256 max_module_id;

        // ── Per-User Global Stats ──────────────────────────────────────────
        /// Total simulation runs (all attempts, including repeat runs).
        mapping(address => uint256) total_simulations;

        /// Count of distinct experiments completed (first-time only).
        mapping(address => uint256) experiments_completed;

        /// Count of learning modules completed.
        mapping(address => uint256) modules_completed;

        // ── Per-User Per-Experiment Stats ──────────────────────────────────
        /// Number of times user ran experiment_id.
        mapping(address => mapping(uint256 => uint256)) run_count;

        /// Whether user has completed experiment_id at least once.
        mapping(address => mapping(uint256 => bool)) experiment_done;

        /// Best score achieved by user for experiment_id (0–100).
        mapping(address => mapping(uint256 => uint256)) best_score;

        // ── Per-User Per-Module Stats ──────────────────────────────────────
        /// Whether user has completed module_id.
        mapping(address => mapping(uint256 => bool)) module_done;

        // ── Global Counters ────────────────────────────────────────────────
        /// Total simulation runs across all users.
        uint256 total_simulations_global;

        /// Total XP awarded through this contract (for analytics).
        uint256 total_xp_awarded;
    }
}
```

**Storage decisions:**
- `score` is stored as `uint256` (0–100) rather than `u8` to stay consistent
  with Stylus StorageUint. Validation enforces `score <= 100`.
- Experiment/module bounds checked against `max_experiment_id` / `max_module_id`
  stored at init. Avoids unbounded mapping growth.
- No per-run history stored on-chain (gas cost). Full simulation history lives
  in event logs, indexed by the backend (see Data Export section).

---

## XP Reward Schedule

```
record_simulation:
  base XP (any run, any score):    50 XP
  perfect score (score == 100):   +50 XP bonus
  first completion of experiment: +25 XP bonus

record_module_completion:
  module completed:               100 XP
```

Rationale: Mirrors DIUReputation's existing XP guide (experiment=100, perfect
quiz=50, daily login=10). A single simulation at 100% = 125 XP (50+50+25),
matching the spirit of "experiment=100 XP" for the first run.

---

## Public Interface

### Initialization

```rust
/// Initialize the contract. Can only be called once.
///
/// - Sets caller as owner and grants initial authorized role.
/// - `reputation_contract`: address of deployed DIUReputation.
///   Pass Address::ZERO to skip XP calls (test/stub mode).
/// - `max_experiment_id`: inclusive upper bound for experiment IDs.
/// - `max_module_id`: inclusive upper bound for module IDs.
pub fn initialize(
    &mut self,
    reputation_contract: Address,
    max_experiment_id: U256,
    max_module_id: U256,
) -> Result<(), ProgressError>
```

### Write Functions (Authorized Only)

```rust
/// Record a simulation run. Authorized callers only.
///
/// - `user`: wallet address of the learner.
/// - `experiment_id`: experiment identifier (0..=max_experiment_id).
/// - `score`: percentage score 0–100. 100 = perfect.
///
/// Awards XP to user via DIUReputation.addXP (skipped if
/// reputation_contract == Address::ZERO).
///
/// Returns total XP awarded in this call.
pub fn record_simulation(
    &mut self,
    user: Address,
    experiment_id: U256,
    score: U256,
) -> Result<U256, ProgressError>

/// Record completion of a learning module. Authorized callers only.
///
/// - `user`: wallet address of the learner.
/// - `module_id`: module identifier (0..=max_module_id).
///
/// Reverts if module was already completed (idempotency guard).
/// Awards 100 XP via DIUReputation.addXP.
pub fn record_module_completion(
    &mut self,
    user: Address,
    module_id: U256,
) -> Result<(), ProgressError>
```

### Admin Functions (Owner Only)

```rust
/// Grant authorized role to a backend address. Owner only.
pub fn grant_authorized(&mut self, account: Address) -> Result<(), ProgressError>

/// Revoke authorized role from an account. Owner only.
pub fn revoke_authorized(&mut self, account: Address) -> Result<(), ProgressError>

/// Update the reputation contract address. Owner only.
///
/// Used when DIUReputation is redeployed (Phase 2 without proxy).
/// Pass Address::ZERO to disable XP calls temporarily.
pub fn set_reputation_contract(
    &mut self,
    new_address: Address,
) -> Result<(), ProgressError>
```

### View Functions (Public)

```rust
/// Get user progress summary.
///
/// Returns: (total_simulations, experiments_completed, modules_completed)
pub fn get_progress_summary(&self, user: Address) -> (U256, U256, U256)

/// Get stats for a single experiment.
///
/// Returns: (run_count, best_score, is_completed)
pub fn get_experiment_stats(
    &self,
    user: Address,
    experiment_id: U256,
) -> (U256, U256, bool)

/// Check whether a module is completed.
pub fn get_module_done(&self, user: Address, module_id: U256) -> bool

/// Get contract owner.
pub fn owner(&self) -> Address

/// Check whether an account is authorized.
pub fn is_authorized(&self, account: Address) -> bool

/// Get address of the DIUReputation contract.
pub fn reputation_contract(&self) -> Address

/// Get global total simulation count across all users.
pub fn total_simulations_global(&self) -> U256

/// Get total XP awarded through this contract.
pub fn total_xp_awarded(&self) -> U256

/// Get max valid experiment_id.
pub fn max_experiment_id(&self) -> U256

/// Get max valid module_id.
pub fn max_module_id(&self) -> U256
```

---

## Data Export Hook (ADR D-019)

ADR D-019 mandates that DIUProgress supports export of simulation results for
the Research Mode extended loop: AI ↔ Simulation ↔ Data Export.

**Two-tier approach:**

### Tier 1 — Event log (primary, gas-efficient)

Every `record_simulation` call emits `SimulationRecorded` with all parameters.
The backend indexes these events into JSON/CSV for Research Mode export. This
is the primary export path — requires no extra storage.

```
event SimulationRecorded(
    address indexed user,
    uint256 indexed experiment_id,
    uint256 score,
    bool is_perfect,
    bool is_first_completion,
    uint256 xp_awarded,
    uint256 run_count_after
)
```

### Tier 2 — On-chain snapshot view (secondary, for direct export)

```rust
/// Get a complete snapshot of experiment progress for a user.
///
/// Returns parallel arrays over experiment_ids 0..=max_experiment_id:
///   (run_counts[], best_scores[], completed_flags[])
///
/// Designed for Research Mode data export and off-chain analytics.
/// Suitable for eth_call (free read). Gas-intensive for large N — use
/// event indexing for production analytics.
pub fn get_export_snapshot(
    &self,
    user: Address,
) -> (Vec<U256>, Vec<U256>, Vec<bool>)
```

This view allows the backend to get a complete user snapshot in a single RPC
call, without needing a full event replay. Maps directly to the JSON export
format: `{ experiments: [{ id, runs, best_score, completed }] }`.

---

## Cross-Contract Call: DIUReputation.addXP

**Interface defined with `sol!`:**

```rust
sol! {
    interface IReputation {
        function addXp(address user, uint256 amount) external;
    }
}
```

**Call site pattern** (in `record_simulation` and `record_module_completion`):

```
let rep_addr = self.reputation_contract.get();
if rep_addr != Address::ZERO {
    // call IReputation::addXp via Stylus SDK call mechanism
    // on failure: emit XpCallFailed event, do NOT revert the record
    // (progress is recorded regardless of XP award failure)
}
```

**Failure handling**: The simulation result is recorded on-chain regardless of
whether the XP cross-contract call succeeds. XP failure emits `XpCallFailed`
event for backend retry. This prevents a buggy/paused reputation contract from
blocking all progress recording.

**Test mode**: `reputation_contract == Address::ZERO` → XP call is skipped
silently. All tests set reputation_contract to zero during `initialize()`.

---

## Events

```rust
sol! {
    /// Emitted when the contract is initialized.
    event Initialized(
        address indexed owner,
        address reputation_contract,
        uint256 max_experiment_id,
        uint256 max_module_id
    );

    /// Emitted when a simulation run is recorded (ADR D-019 export hook).
    event SimulationRecorded(
        address indexed user,
        uint256 indexed experiment_id,
        uint256 score,
        bool is_perfect,
        bool is_first_completion,
        uint256 xp_awarded,
        uint256 run_count_after
    );

    /// Emitted when a learning module is completed.
    event ModuleCompleted(
        address indexed user,
        uint256 indexed module_id,
        uint256 xp_awarded
    );

    /// Emitted when the XP cross-contract call fails (non-reverting).
    event XpCallFailed(address indexed user, uint256 amount);

    /// Emitted when reputation_contract is updated.
    event ReputationContractUpdated(address indexed old_addr, address indexed new_addr);

    /// Emitted when authorized role is granted.
    event AuthorizedGranted(address indexed account, address indexed granted_by);

    /// Emitted when authorized role is revoked.
    event AuthorizedRevoked(address indexed account, address indexed revoked_by);
}
```

---

## Errors

```rust
sol! {
    error Unauthorized();
    error ZeroAddress();
    error AlreadyInitialized();
    error InvalidExperiment();    // experiment_id > max_experiment_id
    error InvalidModule();        // module_id > max_module_id
    error InvalidScore();         // score > 100
    error ModuleAlreadyCompleted();
}
```

Note: No `AlreadyCompleted` error for `record_simulation` — repeat runs are
allowed and encouraged (users retry experiments to improve their score).
Only `record_module_completion` is idempotent-guarded (modules completed once).

---

## Feature Gate (lib.rs / Cargo.toml)

```toml
# Cargo.toml [features]
progress = []
```

```rust
// lib.rs — add after token module
#[cfg(any(test, feature = "progress"))]
pub mod progress;
```

```bash
# Build single contract WASM
cargo stylus check --features progress

# Run all tests (progress tests auto-included)
cargo test
```

---

## Test Plan (TDD — minimum 20 tests)

Target: **24 tests** → total project: **147 + 24 = 171 tests**

### Group 1: Initialization (3 tests)

| # | Test name | What it verifies |
|---|-----------|-----------------|
| 1 | `test_initialize_sets_owner` | `owner()` == caller after init |
| 2 | `test_initialize_twice_reverts` | Second `initialize()` → `AlreadyInitialized` |
| 3 | `test_initialize_sets_reputation_contract` | `reputation_contract()` == passed address |

### Group 2: record_simulation — happy path (4 tests)

| # | Test name | What it verifies |
|---|-----------|-----------------|
| 4 | `test_record_simulation_base_xp` | Score 50 → 50 XP returned; `total_simulations` += 1 |
| 5 | `test_record_simulation_perfect_xp` | Score 100 → 100 XP (50 base + 50 bonus) |
| 6 | `test_record_simulation_first_completion_bonus` | First run on experiment → 75 XP (50+25), `experiments_completed` += 1, `experiment_done` = true |
| 7 | `test_record_simulation_second_run_no_bonus` | Second run on same experiment → 50 XP (no first-completion bonus) |

### Group 3: record_simulation — state updates (4 tests)

| # | Test name | What it verifies |
|---|-----------|-----------------|
| 8 | `test_record_simulation_increments_run_count` | `run_count[user][exp]` increments on every call |
| 9 | `test_record_simulation_updates_best_score` | Lower score after higher → best_score unchanged |
| 10 | `test_record_simulation_best_score_improves` | Higher score → best_score updated |
| 11 | `test_record_simulation_increments_global_counter` | `total_simulations_global()` increments |

### Group 4: record_simulation — error cases (4 tests)

| # | Test name | What it verifies |
|---|-----------|-----------------|
| 12 | `test_record_simulation_unauthorized_reverts` | Non-authorized caller → `Unauthorized` |
| 13 | `test_record_simulation_zero_address_reverts` | `user == Address::ZERO` → `ZeroAddress` |
| 14 | `test_record_simulation_invalid_experiment_reverts` | `experiment_id > max` → `InvalidExperiment` |
| 15 | `test_record_simulation_invalid_score_reverts` | `score > 100` → `InvalidScore` |

### Group 5: record_module_completion (4 tests)

| # | Test name | What it verifies |
|---|-----------|-----------------|
| 16 | `test_module_completion_success` | Awards 100 XP, `module_done` = true, `modules_completed` += 1 |
| 17 | `test_module_completion_already_done_reverts` | Second call → `ModuleAlreadyCompleted` |
| 18 | `test_module_completion_unauthorized_reverts` | Non-authorized → `Unauthorized` |
| 19 | `test_module_completion_invalid_module_reverts` | `module_id > max` → `InvalidModule` |

### Group 6: View functions & export (4 tests)

| # | Test name | What it verifies |
|---|-----------|-----------------|
| 20 | `test_get_progress_summary_default_zeros` | All zeros for unregistered user |
| 21 | `test_get_progress_summary_after_activity` | Summary reflects recorded simulations + modules |
| 22 | `test_get_experiment_stats_accumulate` | Stats correct after multiple runs on same experiment |
| 23 | `test_get_export_snapshot_length` | Snapshot length == max_experiment_id + 1 |

### Group 7: Access control (1 test)

| # | Test name | What it verifies |
|---|-----------|-----------------|
| 24 | `test_revoked_authorized_cannot_record` | Grant → record OK; revoke → record reverts |

---

## Open Questions (pre-implementation)

1. **Proxy pattern** (P-005): If Phase 2 uses a proxy, `set_reputation_contract`
   might be unnecessary (upgradable storage). Defer to Kirill's decision.

2. **XP retry mechanism**: Should failed XP calls (XpCallFailed) be retryable
   on-chain, or is off-chain backend retry sufficient? Current design: backend
   retries via `DIUReputation.addXP` directly.

3. **Experiment/module IDs**: Should these be registered on-chain (registry
   mapping with metadata URIs), or is the numeric ID range sufficient for
   Phase 2? Current design: numeric bounds only, metadata off-chain.

4. **score type**: Using `U256` for consistency with Stylus storage, but `u8`
   would be semantically cleaner. Stylus SDK stores all integers as U256 in
   EVM slots anyway — no storage cost difference.

---

## Implementation Notes

**File to create**: `diu-contracts/src/progress.rs`

**lib.rs changes** (2 lines):
```rust
#[cfg(any(test, feature = "progress"))]
pub mod progress;
```

**Cargo.toml changes** (1 line in `[features]`):
```toml
progress = []
```

**Estimated WASM size**: ~18–22 KB (similar to DIUReputation at 20.0 KB,
slightly larger due to cross-contract call ABI encoding).

**Estimated tests**: 24 (bringing total to 171).
