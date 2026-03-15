//! DIUProgress — simulation runs, module completions, XP cross-contract awards.
//!
//! Tracks per-user physics simulation attempts and learning module completions.
//! Awards XP by calling `DIUReputation.addXp` on each recorded event.
//! Provides a full on-chain snapshot for Research Mode data export (ADR D-019).

use alloc::vec::Vec;
use stylus_sdk::{
    alloy_primitives::{Address, U256},
    alloy_sol_types::sol,
    prelude::*,
};

/// Return type for `get_export_snapshot`: (run_counts, best_scores, completed).
type ExportSnapshot = (Vec<U256>, Vec<U256>, Vec<bool>);

// ═══════════════════════════════════════════════════════════════════════════
// CROSS-CONTRACT INTERFACE
// ═══════════════════════════════════════════════════════════════════════════

sol_interface! {
    /// Minimal interface for the deployed DIUReputation contract.
    interface IReputation {
        function addXp(address user, uint256 amount) external;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Base XP awarded for any simulation run.
const BASE_XP: u64 = 50;

/// Bonus XP for a perfect score (score == 100).
const PERFECT_BONUS_XP: u64 = 50;

/// Bonus XP for the first-ever completion of an experiment.
const FIRST_COMPLETION_XP: u64 = 25;

/// XP awarded for completing a learning module.
const MODULE_XP: u64 = 100;

/// Maximum valid score value (percentage).
const PERFECT_SCORE: u64 = 100;

// ═══════════════════════════════════════════════════════════════════════════
// EVENTS
// ═══════════════════════════════════════════════════════════════════════════

sol! {
    /// Emitted when the contract is initialized.
    event Initialized(
        address indexed owner,
        address reputation_contract,
        uint256 max_experiment_id,
        uint256 max_module_id
    );

    /// Emitted when a simulation run is recorded (ADR D-019 data export hook).
    ///
    /// Contains all parameters needed for backend JSON/CSV export in Research Mode.
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

    /// Emitted when the XP cross-contract call to DIUReputation fails.
    ///
    /// The simulation is recorded regardless; backend should retry addXP directly.
    event XpCallFailed(address indexed user, uint256 amount);

    /// Emitted when the reputation contract address is updated.
    event ReputationContractUpdated(address indexed old_addr, address indexed new_addr);

    /// Emitted when the owner grants authorized role to an account.
    event AuthorizedGranted(address indexed account, address indexed granted_by);

    /// Emitted when the owner revokes authorized role from an account.
    event AuthorizedRevoked(address indexed account, address indexed revoked_by);
}

// ═══════════════════════════════════════════════════════════════════════════
// ERRORS
// ═══════════════════════════════════════════════════════════════════════════

sol! {
    error Unauthorized();
    error ZeroAddress();
    error AlreadyInitialized();
    /// experiment_id > max_experiment_id.
    error InvalidExperiment();
    /// module_id > max_module_id.
    error InvalidModule();
    /// score > 100.
    error InvalidScore();
    /// record_module_completion called twice for the same (user, module_id).
    error ModuleAlreadyCompleted();
}

/// Contract-level error type for DIUProgress.
#[derive(SolidityError)]
pub enum ProgressError {
    Unauthorized(Unauthorized),
    ZeroAddress(ZeroAddress),
    AlreadyInitialized(AlreadyInitialized),
    InvalidExperiment(InvalidExperiment),
    InvalidModule(InvalidModule),
    InvalidScore(InvalidScore),
    ModuleAlreadyCompleted(ModuleAlreadyCompleted),
}

impl core::fmt::Debug for ProgressError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unauthorized(_) => write!(f, "Unauthorized"),
            Self::ZeroAddress(_) => write!(f, "ZeroAddress"),
            Self::AlreadyInitialized(_) => write!(f, "AlreadyInitialized"),
            Self::InvalidExperiment(_) => write!(f, "InvalidExperiment"),
            Self::InvalidModule(_) => write!(f, "InvalidModule"),
            Self::InvalidScore(_) => write!(f, "InvalidScore"),
            Self::ModuleAlreadyCompleted(_) => write!(f, "ModuleAlreadyCompleted"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STORAGE
// ═══════════════════════════════════════════════════════════════════════════

sol_storage! {
    #[entrypoint]
    pub struct DIUProgress {
        // ── Admin ───────────────────────────────────────────────────────────
        /// Contract owner (set during initialize()).
        address owner;

        /// Whether the contract has been initialized.
        bool initialized;

        /// Authorized backend addresses that can record progress.
        mapping(address => bool) authorized;

        /// Address of the deployed DIUReputation contract.
        /// Address::ZERO = skip cross-contract XP calls (test/stub mode).
        address reputation_contract;

        // ── Bounds ──────────────────────────────────────────────────────────
        /// Inclusive upper bound for valid experiment_id values.
        uint256 max_experiment_id;

        /// Inclusive upper bound for valid module_id values.
        uint256 max_module_id;

        // ── Per-User Global Stats ────────────────────────────────────────────
        /// Total simulation runs per user (all attempts including repeats).
        mapping(address => uint256) total_simulations;

        /// Count of distinct experiments first-completed per user.
        mapping(address => uint256) experiments_completed;

        /// Count of learning modules completed per user.
        mapping(address => uint256) modules_completed;

        // ── Per-User Per-Experiment Stats ────────────────────────────────────
        /// Number of times user ran experiment_id.
        mapping(address => mapping(uint256 => uint256)) run_count;

        /// Whether user has completed experiment_id at least once.
        mapping(address => mapping(uint256 => bool)) experiment_done;

        /// Best score (0–100) achieved by user for experiment_id.
        mapping(address => mapping(uint256 => uint256)) best_score;

        // ── Per-User Per-Module Stats ────────────────────────────────────────
        /// Whether user has completed module_id.
        mapping(address => mapping(uint256 => bool)) module_done;

        // ── Global Counters ──────────────────────────────────────────────────
        /// Total simulation runs across all users.
        uint256 total_simulations_global;

        /// Total XP awarded through this contract (for analytics).
        uint256 total_xp_awarded;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INTERNAL HELPERS
// ═══════════════════════════════════════════════════════════════════════════

impl DIUProgress {
    /// Returns an error if the caller is not the contract owner.
    fn require_owner(&self) -> Result<(), ProgressError> {
        if self.vm().msg_sender() != self.owner.get() {
            return Err(ProgressError::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }

    /// Returns an error if the caller is not authorized (owner counts as authorized).
    fn require_authorized(&self) -> Result<(), ProgressError> {
        let caller = self.vm().msg_sender();
        if caller != self.owner.get() && !self.authorized.get(caller) {
            return Err(ProgressError::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }

    /// Try to award XP via DIUReputation.addXp. Non-reverting.
    ///
    /// If the reputation contract address is Address::ZERO (test/stub mode),
    /// the call is silently skipped. If the external call reverts, emits
    /// `XpCallFailed` so the backend can retry directly.
    fn try_award_xp(&mut self, user: Address, amount: U256) {
        let rep_addr = self.reputation_contract.get();
        if rep_addr == Address::ZERO {
            return;
        }
        let reputation = IReputation::new(rep_addr);
        // Call::new_mutating requires &mut TopLevelStorage (satisfied by #[entrypoint]).
        // It does not hold the borrow after returning, so self.vm() is safe afterward.
        let cfg = Call::new_mutating(self);
        if reputation.add_xp(self.vm(), cfg, user, amount).is_err() {
            self.vm().log(XpCallFailed { user, amount });
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC INTERFACE
// ═══════════════════════════════════════════════════════════════════════════

#[public]
impl DIUProgress {
    // ─── Initialization ──────────────────────────────────────────────

    /// Initialize the contract. Can only be called once.
    ///
    /// Sets the caller as owner and grants initial authorized role.
    ///
    /// - `reputation_contract`: deployed DIUReputation address.
    ///   Pass `Address::ZERO` to disable XP calls (test/stub mode).
    /// - `max_experiment_id`: inclusive upper bound for experiment IDs.
    /// - `max_module_id`: inclusive upper bound for module IDs.
    pub fn initialize(
        &mut self,
        reputation_contract: Address,
        max_experiment_id: U256,
        max_module_id: U256,
    ) -> Result<(), ProgressError> {
        if self.initialized.get() {
            return Err(ProgressError::AlreadyInitialized(AlreadyInitialized {}));
        }

        let caller = self.vm().msg_sender();
        self.owner.set(caller);
        self.authorized.setter(caller).set(true);
        self.initialized.set(true);
        self.reputation_contract.set(reputation_contract);
        self.max_experiment_id.set(max_experiment_id);
        self.max_module_id.set(max_module_id);

        self.vm().log(Initialized {
            owner: caller,
            reputation_contract,
            max_experiment_id,
            max_module_id,
        });

        Ok(())
    }

    // ─── Write Functions (Authorized Only) ───────────────────────────

    /// Record a simulation run and award XP. Authorized callers only.
    ///
    /// `experiment_id` must be <= `max_experiment_id`.
    /// `score` must be in range 0–100 (percentage; 100 = perfect).
    ///
    /// XP schedule per call:
    /// - Any run:           50 XP (base)
    /// - Perfect (== 100): +50 XP
    /// - First completion: +25 XP
    ///
    /// XP is awarded via `DIUReputation.addXp` (skipped if reputation_contract
    /// is `Address::ZERO`). Failures emit `XpCallFailed` without reverting.
    ///
    /// Returns the total XP awarded in this call.
    pub fn record_simulation(
        &mut self,
        user: Address,
        experiment_id: U256,
        score: U256,
    ) -> Result<U256, ProgressError> {
        self.require_authorized()?;

        if user == Address::ZERO {
            return Err(ProgressError::ZeroAddress(ZeroAddress {}));
        }
        if experiment_id > self.max_experiment_id.get() {
            return Err(ProgressError::InvalidExperiment(InvalidExperiment {}));
        }
        if score > U256::from(PERFECT_SCORE) {
            return Err(ProgressError::InvalidScore(InvalidScore {}));
        }

        // ── XP calculation ───────────────────────────────────────────
        let mut xp: u64 = BASE_XP;

        let is_perfect = score == U256::from(PERFECT_SCORE);
        if is_perfect {
            xp += PERFECT_BONUS_XP;
        }

        let is_first = !self.experiment_done.get(user).get(experiment_id);
        if is_first {
            xp += FIRST_COMPLETION_XP;
            self.experiment_done.setter(user).setter(experiment_id).set(true);
            let prev_exps = self.experiments_completed.get(user);
            self.experiments_completed
                .setter(user)
                .set(prev_exps + U256::from(1));
        }

        // ── Best score ───────────────────────────────────────────────
        let current_best = self.best_score.get(user).get(experiment_id);
        if score > current_best {
            self.best_score.setter(user).setter(experiment_id).set(score);
        }

        // ── Run counters ─────────────────────────────────────────────
        let runs = self.run_count.get(user).get(experiment_id);
        let new_runs = runs + U256::from(1);
        self.run_count.setter(user).setter(experiment_id).set(new_runs);

        let total = self.total_simulations.get(user);
        self.total_simulations.setter(user).set(total + U256::from(1));

        let global = self.total_simulations_global.get();
        self.total_simulations_global.set(global + U256::from(1));

        // ── XP accounting ────────────────────────────────────────────
        let xp_u256 = U256::from(xp);
        let prev_xp = self.total_xp_awarded.get();
        self.total_xp_awarded.set(prev_xp + xp_u256);

        self.vm().log(SimulationRecorded {
            user,
            experiment_id,
            score,
            is_perfect,
            is_first_completion: is_first,
            xp_awarded: xp_u256,
            run_count_after: new_runs,
        });

        // ── Cross-contract XP award (non-reverting) ──────────────────
        self.try_award_xp(user, xp_u256);

        Ok(xp_u256)
    }

    /// Record completion of a learning module. Authorized callers only.
    ///
    /// `module_id` must be <= `max_module_id`.
    /// Reverts if the module was already completed (idempotency guard).
    /// Awards 100 XP via `DIUReputation.addXp`.
    pub fn record_module_completion(
        &mut self,
        user: Address,
        module_id: U256,
    ) -> Result<(), ProgressError> {
        self.require_authorized()?;

        if user == Address::ZERO {
            return Err(ProgressError::ZeroAddress(ZeroAddress {}));
        }
        if module_id > self.max_module_id.get() {
            return Err(ProgressError::InvalidModule(InvalidModule {}));
        }
        if self.module_done.get(user).get(module_id) {
            return Err(ProgressError::ModuleAlreadyCompleted(
                ModuleAlreadyCompleted {},
            ));
        }

        self.module_done.setter(user).setter(module_id).set(true);

        let prev = self.modules_completed.get(user);
        self.modules_completed.setter(user).set(prev + U256::from(1));

        let xp = U256::from(MODULE_XP);
        let prev_xp = self.total_xp_awarded.get();
        self.total_xp_awarded.set(prev_xp + xp);

        self.vm().log(ModuleCompleted {
            user,
            module_id,
            xp_awarded: xp,
        });

        self.try_award_xp(user, xp);

        Ok(())
    }

    // ─── Access Control (Owner Only) ─────────────────────────────────

    /// Grant authorized role to a backend address. Owner only.
    pub fn grant_authorized(&mut self, account: Address) -> Result<(), ProgressError> {
        self.require_owner()?;

        if account == Address::ZERO {
            return Err(ProgressError::ZeroAddress(ZeroAddress {}));
        }

        self.authorized.setter(account).set(true);

        let granted_by = self.vm().msg_sender();
        self.vm().log(AuthorizedGranted { account, granted_by });

        Ok(())
    }

    /// Revoke authorized role from an account. Owner only.
    pub fn revoke_authorized(&mut self, account: Address) -> Result<(), ProgressError> {
        self.require_owner()?;

        self.authorized.setter(account).set(false);

        let revoked_by = self.vm().msg_sender();
        self.vm().log(AuthorizedRevoked { account, revoked_by });

        Ok(())
    }

    /// Update the DIUReputation contract address. Owner only.
    ///
    /// Used when DIUReputation is redeployed (Phase 2 without proxy).
    /// Pass `Address::ZERO` to temporarily disable XP calls.
    pub fn set_reputation_contract(
        &mut self,
        new_address: Address,
    ) -> Result<(), ProgressError> {
        self.require_owner()?;

        let old = self.reputation_contract.get();
        self.reputation_contract.set(new_address);

        self.vm().log(ReputationContractUpdated {
            old_addr: old,
            new_addr: new_address,
        });

        Ok(())
    }

    // ─── View Functions ──────────────────────────────────────────────

    /// Get user progress summary.
    ///
    /// Returns `(total_simulations, experiments_completed, modules_completed)`.
    pub fn get_progress_summary(&self, user: Address) -> (U256, U256, U256) {
        (
            self.total_simulations.get(user),
            self.experiments_completed.get(user),
            self.modules_completed.get(user),
        )
    }

    /// Get stats for a single experiment.
    ///
    /// Returns `(run_count, best_score, is_completed)`.
    pub fn get_experiment_stats(
        &self,
        user: Address,
        experiment_id: U256,
    ) -> (U256, U256, bool) {
        (
            self.run_count.get(user).get(experiment_id),
            self.best_score.get(user).get(experiment_id),
            self.experiment_done.get(user).get(experiment_id),
        )
    }

    /// Check whether a module is completed by a user.
    pub fn get_module_done(&self, user: Address, module_id: U256) -> bool {
        self.module_done.get(user).get(module_id)
    }

    /// Get a full snapshot of all experiment progress for data export (ADR D-019).
    ///
    /// Returns parallel arrays over experiment IDs `0..=max_experiment_id`:
    /// `(run_counts[], best_scores[], completed_flags[])`.
    ///
    /// Authorized callers only (P-3 security fix): prevents unrestricted
    /// enumeration of user progress data (GDPR / privacy concern).
    ///
    /// For `max_experiment_id` > 50, prefer event-based indexing instead.
    pub fn get_export_snapshot(
        &self,
        user: Address,
    ) -> Result<ExportSnapshot, ProgressError> {
        self.require_authorized()?;

        let max = self.max_experiment_id.get().saturating_to::<usize>();
        let len = max + 1; // experiments 0..=max

        let mut run_counts = Vec::with_capacity(len);
        let mut best_scores = Vec::with_capacity(len);
        let mut completed = Vec::with_capacity(len);

        for i in 0..len {
            let exp_id = U256::from(i);
            run_counts.push(self.run_count.get(user).get(exp_id));
            best_scores.push(self.best_score.get(user).get(exp_id));
            completed.push(self.experiment_done.get(user).get(exp_id));
        }

        Ok((run_counts, best_scores, completed))
    }

    /// Get the contract owner address.
    pub fn owner(&self) -> Address {
        self.owner.get()
    }

    /// Check whether an account is authorized (owner always counts as authorized).
    pub fn is_authorized(&self, account: Address) -> bool {
        account == self.owner.get() || self.authorized.get(account)
    }

    /// Get the DIUReputation contract address used for XP calls.
    pub fn reputation_contract(&self) -> Address {
        self.reputation_contract.get()
    }

    /// Get total simulation runs across all users.
    pub fn total_simulations_global(&self) -> U256 {
        self.total_simulations_global.get()
    }

    /// Get total XP awarded through this contract (for analytics).
    pub fn total_xp_awarded(&self) -> U256 {
        self.total_xp_awarded.get()
    }

    /// Get the inclusive upper bound for valid experiment IDs.
    pub fn max_experiment_id(&self) -> U256 {
        self.max_experiment_id.get()
    }

    /// Get the inclusive upper bound for valid module IDs.
    pub fn max_module_id(&self) -> U256 {
        self.max_module_id.get()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::address;
    use stylus_sdk::testing::*;

    const OWNER: Address = address!("1111111111111111111111111111111111111111");
    const BACKEND: Address = address!("2222222222222222222222222222222222222222");
    const ALICE: Address = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    const BOB: Address = address!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

    // max_experiment_id = 10 (valid: 0..=10)
    // max_module_id     = 5  (valid: 0..=5)
    fn max_exp() -> U256 {
        U256::from(10)
    }
    fn max_mod() -> U256 {
        U256::from(5)
    }

    /// Helper: initialized contract with OWNER as owner.
    /// Uses Address::ZERO for reputation_contract to skip cross-contract calls.
    fn setup() -> (TestVM, DIUProgress) {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIUProgress::from(&vm);
        contract
            .initialize(Address::ZERO, max_exp(), max_mod())
            .unwrap();
        (vm, contract)
    }

    /// Helper: setup with an authorized BACKEND address; switches sender to BACKEND.
    fn setup_with_backend() -> (TestVM, DIUProgress) {
        let (vm, mut contract) = setup();
        contract.grant_authorized(BACKEND).unwrap();
        vm.set_sender(BACKEND);
        (vm, contract)
    }

    // ─── Group 1: Initialization (3 tests) ──────────────────────────

    #[test]
    fn test_initialize_sets_owner() {
        let (_, contract) = setup();
        assert_eq!(contract.owner(), OWNER);
    }

    #[test]
    fn test_initialize_twice_reverts() {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIUProgress::from(&vm);
        contract
            .initialize(Address::ZERO, max_exp(), max_mod())
            .unwrap();

        let result = contract.initialize(Address::ZERO, max_exp(), max_mod());
        assert!(matches!(result, Err(ProgressError::AlreadyInitialized(_))));
    }

    #[test]
    fn test_initialize_sets_reputation_contract() {
        let rep = address!("cccccccccccccccccccccccccccccccccccccccc");
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIUProgress::from(&vm);
        contract.initialize(rep, max_exp(), max_mod()).unwrap();
        assert_eq!(contract.reputation_contract(), rep);
    }

    // ─── Group 2: record_simulation — XP calculation (4 tests) ──────

    #[test]
    fn test_record_simulation_first_run_xp() {
        // First run, non-perfect: base(50) + first_completion(25) = 75 XP
        let (_, mut contract) = setup_with_backend();
        let xp = contract
            .record_simulation(ALICE, U256::from(0), U256::from(50))
            .unwrap();
        assert_eq!(xp, U256::from(75));
        assert_eq!(contract.total_xp_awarded(), U256::from(75));
    }

    #[test]
    fn test_record_simulation_second_run_xp() {
        // Second run: base(50) only — no first_completion bonus
        let (_, mut contract) = setup_with_backend();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(50))
            .unwrap();
        let xp = contract
            .record_simulation(ALICE, U256::from(0), U256::from(50))
            .unwrap();
        assert_eq!(xp, U256::from(50));
    }

    #[test]
    fn test_record_simulation_perfect_first_run_xp() {
        // First run, perfect: base(50) + perfect(50) + first_completion(25) = 125 XP
        let (_, mut contract) = setup_with_backend();
        let xp = contract
            .record_simulation(ALICE, U256::from(0), U256::from(100))
            .unwrap();
        assert_eq!(xp, U256::from(125));
    }

    #[test]
    fn test_record_simulation_perfect_second_run_xp() {
        // Perfect second run: base(50) + perfect(50) = 100 XP
        let (_, mut contract) = setup_with_backend();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(50))
            .unwrap();
        let xp = contract
            .record_simulation(ALICE, U256::from(0), U256::from(100))
            .unwrap();
        assert_eq!(xp, U256::from(100));
    }

    // ─── Group 3: record_simulation — state updates (4 tests) ───────

    #[test]
    fn test_record_simulation_increments_run_count() {
        let (_, mut contract) = setup_with_backend();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(50))
            .unwrap();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(60))
            .unwrap();

        let (run_count, _, _) = contract.get_experiment_stats(ALICE, U256::from(0));
        assert_eq!(run_count, U256::from(2));
    }

    #[test]
    fn test_record_simulation_best_score_does_not_regress() {
        let (_, mut contract) = setup_with_backend();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(80))
            .unwrap();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(40))
            .unwrap();

        let (_, best_score, _) = contract.get_experiment_stats(ALICE, U256::from(0));
        assert_eq!(best_score, U256::from(80));
    }

    #[test]
    fn test_record_simulation_best_score_improves() {
        let (_, mut contract) = setup_with_backend();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(40))
            .unwrap();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(80))
            .unwrap();

        let (_, best_score, _) = contract.get_experiment_stats(ALICE, U256::from(0));
        assert_eq!(best_score, U256::from(80));
    }

    #[test]
    fn test_record_simulation_increments_global_counter() {
        let (_, mut contract) = setup_with_backend();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(50))
            .unwrap();
        contract
            .record_simulation(ALICE, U256::from(1), U256::from(50))
            .unwrap();

        assert_eq!(contract.total_simulations_global(), U256::from(2));
    }

    // ─── Group 4: record_simulation — error cases (4 tests) ─────────

    #[test]
    fn test_record_simulation_unauthorized_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.record_simulation(BOB, U256::from(0), U256::from(50));
        assert!(matches!(result, Err(ProgressError::Unauthorized(_))));
    }

    #[test]
    fn test_record_simulation_zero_address_reverts() {
        let (_, mut contract) = setup_with_backend();

        let result = contract.record_simulation(Address::ZERO, U256::from(0), U256::from(50));
        assert!(matches!(result, Err(ProgressError::ZeroAddress(_))));
    }

    #[test]
    fn test_record_simulation_invalid_experiment_reverts() {
        // max_experiment_id = 10; experiment 11 is out of range
        let (_, mut contract) = setup_with_backend();

        let result = contract.record_simulation(ALICE, U256::from(11), U256::from(50));
        assert!(matches!(result, Err(ProgressError::InvalidExperiment(_))));
    }

    #[test]
    fn test_record_simulation_invalid_score_reverts() {
        let (_, mut contract) = setup_with_backend();

        let result = contract.record_simulation(ALICE, U256::from(0), U256::from(101));
        assert!(matches!(result, Err(ProgressError::InvalidScore(_))));
    }

    // ─── Group 5: record_module_completion (4 tests) ─────────────────

    #[test]
    fn test_module_completion_success() {
        let (_, mut contract) = setup_with_backend();
        contract
            .record_module_completion(ALICE, U256::from(0))
            .unwrap();

        assert!(contract.get_module_done(ALICE, U256::from(0)));
        let (_, _, mods) = contract.get_progress_summary(ALICE);
        assert_eq!(mods, U256::from(1));
        assert_eq!(contract.total_xp_awarded(), U256::from(MODULE_XP));
    }

    #[test]
    fn test_module_completion_already_done_reverts() {
        let (_, mut contract) = setup_with_backend();
        contract
            .record_module_completion(ALICE, U256::from(0))
            .unwrap();

        let result = contract.record_module_completion(ALICE, U256::from(0));
        assert!(matches!(
            result,
            Err(ProgressError::ModuleAlreadyCompleted(_))
        ));
    }

    #[test]
    fn test_module_completion_unauthorized_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.record_module_completion(BOB, U256::from(0));
        assert!(matches!(result, Err(ProgressError::Unauthorized(_))));
    }

    #[test]
    fn test_module_completion_invalid_module_reverts() {
        // max_module_id = 5; module 6 is out of range
        let (_, mut contract) = setup_with_backend();

        let result = contract.record_module_completion(ALICE, U256::from(6));
        assert!(matches!(result, Err(ProgressError::InvalidModule(_))));
    }

    // ─── Group 6: View functions & export (4 tests) ──────────────────

    #[test]
    fn test_get_progress_summary_default_zeros() {
        let (_, contract) = setup();

        let (sims, exps, mods) = contract.get_progress_summary(ALICE);
        assert_eq!(sims, U256::ZERO);
        assert_eq!(exps, U256::ZERO);
        assert_eq!(mods, U256::ZERO);
    }

    #[test]
    fn test_get_progress_summary_after_activity() {
        let (_, mut contract) = setup_with_backend();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(80))
            .unwrap();
        contract
            .record_simulation(ALICE, U256::from(1), U256::from(60))
            .unwrap();
        contract
            .record_module_completion(ALICE, U256::from(0))
            .unwrap();

        let (sims, exps, mods) = contract.get_progress_summary(ALICE);
        assert_eq!(sims, U256::from(2));
        assert_eq!(exps, U256::from(2)); // 2 distinct first-completions
        assert_eq!(mods, U256::from(1));
    }

    #[test]
    fn test_get_experiment_stats_accumulate() {
        let (_, mut contract) = setup_with_backend();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(60))
            .unwrap();
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(80))
            .unwrap();

        let (run_count, best_score, completed) =
            contract.get_experiment_stats(ALICE, U256::from(0));
        assert_eq!(run_count, U256::from(2));
        assert_eq!(best_score, U256::from(80));
        assert!(completed);
    }

    #[test]
    fn test_get_export_snapshot_length() {
        let (_, contract) = setup();
        // max_experiment_id = 10 → arrays have 11 elements (0..=10)
        // Owner is authorized by default.
        let (run_counts, best_scores, completed) =
            contract.get_export_snapshot(ALICE).unwrap();
        assert_eq!(run_counts.len(), 11);
        assert_eq!(best_scores.len(), 11);
        assert_eq!(completed.len(), 11);
    }

    #[test]
    fn test_get_export_snapshot_unauthorized_reverts() {
        let (vm, contract) = setup();
        vm.set_sender(ALICE); // not authorized
        let result = contract.get_export_snapshot(ALICE);
        assert!(matches!(result, Err(ProgressError::Unauthorized(_))));
    }

    // ─── Group 7: Access control (1 test) ────────────────────────────

    #[test]
    fn test_revoked_authorized_cannot_record() {
        let (vm, mut contract) = setup();

        contract.grant_authorized(BACKEND).unwrap();
        vm.set_sender(BACKEND);
        contract
            .record_simulation(ALICE, U256::from(0), U256::from(50))
            .unwrap();

        vm.set_sender(OWNER);
        contract.revoke_authorized(BACKEND).unwrap();

        vm.set_sender(BACKEND);
        let result = contract.record_simulation(ALICE, U256::from(0), U256::from(50));
        assert!(matches!(result, Err(ProgressError::Unauthorized(_))));
    }
}
