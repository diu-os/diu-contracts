//! DIUProgress — simulation runs, module completions, XP cross-contract awards.
//!
//! Tracks per-user physics simulation attempts and learning module completions.
//! Awards XP by calling `DIUReputation.addXp` on each recorded event.
//! Provides a full on-chain snapshot for Research Mode data export (ADR D-019).
//!
//! v2 (ADR D-087): on-chain result attestation. A `content_hash` (RFC 8785
//! canonical manifest hash, ADR D-086) is anchored on-chain either by the
//! attester itself (`attest_result`, self-custody) or by a relayer carrying the
//! attester's EIP-712 signature (`attest_with_sig`, meta-tx). Both paths respect
//! the universal pause (ADR D-029).

use crate::pause::{ContractNotPaused, ContractPaused, PauseError, PauseStorage, Paused, Unpaused};
use alloc::vec::Vec;
use stylus_sdk::{
    alloy_primitives::{Address, Bytes, FixedBytes, B256, U256},
    alloy_sol_types::sol,
    call::static_call,
    crypto,
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
        /// Award XP; nonce must match the per-user counter (ADR D-026).
        function addXp(address user, uint256 amount, uint64 nonce) external;
        /// Return the current replay-prevention nonce for a user (ADR D-026).
        function getNonce(address user) external view returns (uint64);
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

/// Storage layout version of this deployment (ADR D-025 discipline; v2 = ADR D-087).
const STORAGE_VERSION: u64 = 2;

/// EVM `ecrecover` precompile address (0x0000…0001).
///
/// Stylus SDK 0.10 has no native ecrecover, so recovery is a static call to the
/// precompile (ADR D-087).
const ECRECOVER_PRECOMPILE: Address = Address::with_last_byte(0x01);

/// EIP-712 domain type string (ADR D-087).
const EIP712_DOMAIN_TYPE: &[u8] =
    b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";

/// EIP-712 `Attest` struct type string (ADR D-087).
const ATTEST_TYPE: &[u8] =
    b"Attest(bytes32 contentHash,address attester,uint256 nonce,uint256 deadline)";

/// EIP-712 domain name (ADR D-087).
const EIP712_DOMAIN_NAME: &[u8] = b"DIUProgress";

/// EIP-712 domain version (ADR D-087).
const EIP712_DOMAIN_VERSION: &[u8] = b"2";

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

    /// Emitted when a result attestation is recorded on-chain (ADR D-087).
    ///
    /// No module_id by design (D-087 O-2 resolved): the content_hash is the
    /// single self-sufficient join key; module indexing happens off-chain (P-012).
    event AttestationRecorded(
        bytes32 indexed content_hash,
        address indexed attester,
        uint64 timestamp,
        bool via_relay
    );

    /// Emitted when contract ownership is transferred (ADR D-087 §6: → Gnosis Safe).
    event OwnershipTransferred(address indexed previous_owner, address indexed new_owner);
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
    /// (content_hash, attester) pair already attested (ADR D-087).
    error AlreadyAttested();
    /// ecrecover did not yield the claimed attester (ADR D-087).
    error InvalidSignature();
    /// block.timestamp > deadline (ADR D-087).
    error ExpiredSignature();
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
    AlreadyAttested(AlreadyAttested),
    InvalidSignature(InvalidSignature),
    ExpiredSignature(ExpiredSignature),
    ContractPaused(ContractPaused),
    ContractNotPaused(ContractNotPaused),
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
            Self::AlreadyAttested(_) => write!(f, "AlreadyAttested"),
            Self::InvalidSignature(_) => write!(f, "InvalidSignature"),
            Self::ExpiredSignature(_) => write!(f, "ExpiredSignature"),
            Self::ContractPaused(_) => write!(f, "ContractPaused"),
            Self::ContractNotPaused(_) => write!(f, "ContractNotPaused"),
        }
    }
}

impl From<PauseError> for ProgressError {
    fn from(e: PauseError) -> Self {
        match e {
            PauseError::ContractPaused(e) => Self::ContractPaused(e),
            PauseError::NotPauseGuardian(_) => Self::Unauthorized(Unauthorized {}),
            PauseError::ContractNotPaused(e) => Self::ContractNotPaused(e),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STORAGE
// ═══════════════════════════════════════════════════════════════════════════

sol_storage! {
    #[entrypoint]
    pub struct DIUProgress {
        // ── Layout version (ADR D-025: first field, frozen) ──────────────────
        /// Storage layout version. = 2 for this deployment (ADR D-087).
        uint256 storage_version;

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

        // ── Pause state (ADR D-029) ──────────────────────────────────────────
        /// Pause state: paused flag + guardian address.
        PauseStorage pause_state;

        // ── Attestations (ADR D-087, new in v2 — strictly appended) ──────────
        /// Number of distinct addresses that attested a content_hash.
        mapping(bytes32 => uint256) attestation_count;

        /// Whether (content_hash, attester) has been attested.
        mapping(bytes32 => mapping(address => bool)) attested;

        /// Per-attester meta-tx nonce for attest_with_sig (anti-replay, D-026).
        mapping(address => uint256) relay_nonces;

        // ── Reserved (ADR D-025) ─────────────────────────────────────────────
        uint256 _reserved0;
        uint256 _reserved1;
        uint256 _reserved2;
        uint256 _reserved3;
        uint256 _reserved4;
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
    ///
    /// Implements ADR D-026: fetches the current per-user nonce via a static
    /// call, then passes it to addXp. Both calls are within the same transaction
    /// so no other tx can interleave and invalidate the nonce.
    fn try_award_xp(&mut self, user: Address, amount: U256) {
        let rep_addr = self.reputation_contract.get();
        if rep_addr == Address::ZERO {
            return;
        }
        let reputation = IReputation::new(rep_addr);

        // Static call: read current nonce (no state change).
        let nonce = match reputation.get_nonce(self.vm(), Call::new(), user) {
            Ok(n) => n,
            Err(_) => {
                self.vm().log(XpCallFailed { user, amount });
                return;
            }
        };

        // Mutating call: award XP with the fetched nonce.
        // Call::new_mutating requires &mut TopLevelStorage (satisfied by #[entrypoint]).
        let cfg = Call::new_mutating(self);
        if reputation
            .add_xp(self.vm(), cfg, user, amount, nonce)
            .is_err()
        {
            self.vm().log(XpCallFailed { user, amount });
        }
    }

    // ─── Attestation helpers (ADR D-087) ─────────────────────────────

    /// Compute the EIP-712 digest for `Attest(contentHash, attester, nonce, deadline)`.
    ///
    /// Domain: `{ name: "DIUProgress", version: "2", chainId, verifyingContract }`.
    /// `nonce` is the contract-side `relay_nonces[attester]` value — it is never
    /// a caller parameter, which is what makes a signature single-use (ADR D-026).
    fn attest_digest(
        &self,
        content_hash: FixedBytes<32>,
        attester: Address,
        nonce: U256,
        deadline: U256,
    ) -> B256 {
        let domain_separator = {
            let mut buf = [0u8; 160];
            buf[0..32].copy_from_slice(crypto::keccak(EIP712_DOMAIN_TYPE).as_slice());
            buf[32..64].copy_from_slice(crypto::keccak(EIP712_DOMAIN_NAME).as_slice());
            buf[64..96].copy_from_slice(crypto::keccak(EIP712_DOMAIN_VERSION).as_slice());
            buf[96..128].copy_from_slice(&U256::from(self.vm().chain_id()).to_be_bytes::<32>());
            buf[128..160].copy_from_slice(
                B256::left_padding_from(self.vm().contract_address().as_slice()).as_slice(),
            );
            crypto::keccak(buf)
        };

        let struct_hash = {
            let mut buf = [0u8; 160];
            buf[0..32].copy_from_slice(crypto::keccak(ATTEST_TYPE).as_slice());
            buf[32..64].copy_from_slice(content_hash.as_slice());
            buf[64..96].copy_from_slice(B256::left_padding_from(attester.as_slice()).as_slice());
            buf[96..128].copy_from_slice(&nonce.to_be_bytes::<32>());
            buf[128..160].copy_from_slice(&deadline.to_be_bytes::<32>());
            crypto::keccak(buf)
        };

        let mut preimage = [0u8; 66];
        preimage[0] = 0x19;
        preimage[1] = 0x01;
        preimage[2..34].copy_from_slice(domain_separator.as_slice());
        preimage[34..66].copy_from_slice(struct_hash.as_slice());
        crypto::keccak(preimage)
    }

    /// Recover the signer of `digest` from a 65-byte `r||s||v` signature.
    ///
    /// Stylus SDK 0.10 has no native ecrecover — this is a static call to the
    /// 0x01 precompile, which returns empty output for invalid signatures.
    fn recover_signer(&self, digest: B256, signature: &[u8]) -> Result<Address, ProgressError> {
        if signature.len() != 65 {
            return Err(ProgressError::InvalidSignature(InvalidSignature {}));
        }
        let mut v = signature[64];
        if v < 27 {
            // Normalise 0/1 recovery ids to the precompile's expected 27/28.
            v += 27;
        }

        let mut input = [0u8; 128];
        input[0..32].copy_from_slice(digest.as_slice());
        input[63] = v;
        input[64..128].copy_from_slice(&signature[0..64]);

        let output = static_call(self.vm(), Call::new(), ECRECOVER_PRECOMPILE, &input)
            .map_err(|_| ProgressError::InvalidSignature(InvalidSignature {}))?;

        if output.len() != 32 {
            return Err(ProgressError::InvalidSignature(InvalidSignature {}));
        }
        let recovered = Address::from_slice(&output[12..32]);
        if recovered == Address::ZERO {
            return Err(ProgressError::InvalidSignature(InvalidSignature {}));
        }
        Ok(recovered)
    }

    /// Record an attestation for `(content_hash, attester)` and emit the event.
    ///
    /// Shared tail of `attest_result` and `attest_with_sig` (ADR D-087).
    fn record_attestation(
        &mut self,
        content_hash: FixedBytes<32>,
        attester: Address,
        via_relay: bool,
    ) -> Result<(), ProgressError> {
        if self.attested.get(content_hash).get(attester) {
            return Err(ProgressError::AlreadyAttested(AlreadyAttested {}));
        }
        self.attested
            .setter(content_hash)
            .setter(attester)
            .set(true);

        let count = self.attestation_count.get(content_hash);
        self.attestation_count
            .setter(content_hash)
            .set(count + U256::from(1));

        self.vm().log(AttestationRecorded {
            content_hash,
            attester,
            timestamp: self.vm().block_timestamp(),
            via_relay,
        });

        Ok(())
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
        self.storage_version.set(U256::from(STORAGE_VERSION));
        self.owner.set(caller);
        self.authorized.setter(caller).set(true);
        self.initialized.set(true);
        self.reputation_contract.set(reputation_contract);
        self.max_experiment_id.set(max_experiment_id);
        self.max_module_id.set(max_module_id);
        self.pause_state.set_pause_guardian(caller);

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
            self.experiment_done
                .setter(user)
                .setter(experiment_id)
                .set(true);
            let prev_exps = self.experiments_completed.get(user);
            self.experiments_completed
                .setter(user)
                .set(prev_exps + U256::from(1));
        }

        // ── Best score ───────────────────────────────────────────────
        let current_best = self.best_score.get(user).get(experiment_id);
        if score > current_best {
            self.best_score
                .setter(user)
                .setter(experiment_id)
                .set(score);
        }

        // ── Run counters ─────────────────────────────────────────────
        let runs = self.run_count.get(user).get(experiment_id);
        let new_runs = runs + U256::from(1);
        self.run_count
            .setter(user)
            .setter(experiment_id)
            .set(new_runs);

        let total = self.total_simulations.get(user);
        self.total_simulations
            .setter(user)
            .set(total + U256::from(1));

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
        self.modules_completed
            .setter(user)
            .set(prev + U256::from(1));

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

    // ─── Attestation (ADR D-087) ──────────────────────────────────────

    /// Attest a result `content_hash` as `msg.sender` (self-custody path).
    ///
    /// Any address may attest any hash — ownership of the underlying export is
    /// an off-chain concept. Multiple attestations of one hash by different
    /// addresses signal independent replication; a repeat by the same address
    /// reverts with `AlreadyAttested`.
    pub fn attest_result(&mut self, content_hash: FixedBytes<32>) -> Result<(), ProgressError> {
        self.pause_state.require_not_paused()?;
        let attester = self.vm().msg_sender();
        self.record_attestation(content_hash, attester, false)
    }

    /// Attest a result on behalf of `attester` via an EIP-712 signature (meta-tx).
    ///
    /// The relayer pays gas; authorship is proven by the attester's signature
    /// over `Attest(contentHash, attester, nonce, deadline)`. The nonce is read
    /// from `relay_nonces[attester]` and included in the digest, so each
    /// signature is single-use; a stale nonce or altered field fails as
    /// `InvalidSignature`. `signature` is 65 bytes `r||s||v`.
    pub fn attest_with_sig(
        &mut self,
        content_hash: FixedBytes<32>,
        attester: Address,
        deadline: U256,
        signature: Bytes,
    ) -> Result<(), ProgressError> {
        self.pause_state.require_not_paused()?;

        if U256::from(self.vm().block_timestamp()) > deadline {
            return Err(ProgressError::ExpiredSignature(ExpiredSignature {}));
        }

        let nonce = self.relay_nonces.get(attester);
        let digest = self.attest_digest(content_hash, attester, nonce, deadline);
        let recovered = self.recover_signer(digest, &signature)?;
        if recovered != attester {
            return Err(ProgressError::InvalidSignature(InvalidSignature {}));
        }

        self.relay_nonces
            .setter(attester)
            .set(nonce + U256::from(1));
        self.record_attestation(content_hash, attester, true)
    }

    // ─── Pause (ADR D-029) ────────────────────────────────────────────

    /// Pause the attestation functions. Pause guardian only.
    pub fn pause(&mut self) -> Result<(), ProgressError> {
        let caller = self.vm().msg_sender();
        self.pause_state.pause(caller)?;
        self.vm().log(Paused { guardian: caller });
        Ok(())
    }

    /// Unpause the contract. Pause guardian only.
    pub fn unpause(&mut self) -> Result<(), ProgressError> {
        let caller = self.vm().msg_sender();
        self.pause_state.unpause(caller)?;
        self.vm().log(Unpaused { guardian: caller });
        Ok(())
    }

    /// Set the pause guardian address. Owner only (ADR D-029).
    pub fn set_pause_guardian(&mut self, guardian: Address) -> Result<(), ProgressError> {
        self.require_owner()?;
        if guardian == Address::ZERO {
            return Err(ProgressError::ZeroAddress(ZeroAddress {}));
        }
        self.pause_state.set_pause_guardian(guardian);
        Ok(())
    }

    /// Check whether the contract is paused.
    pub fn is_paused(&self) -> bool {
        self.pause_state.is_paused()
    }

    /// Get the current pause guardian address.
    pub fn get_pause_guardian(&self) -> Address {
        self.pause_state.get_pause_guardian()
    }

    // ─── Access Control (Owner Only) ─────────────────────────────────

    /// Transfer contract ownership. Owner only.
    ///
    /// ADR D-087 §6: ownership moves to the Gnosis Safe 2-of-3 (D-027) right
    /// after deploy; the contract is immutable, so the capability ships in v2.
    pub fn transfer_ownership(&mut self, new_owner: Address) -> Result<(), ProgressError> {
        self.require_owner()?;
        if new_owner == Address::ZERO {
            return Err(ProgressError::ZeroAddress(ZeroAddress {}));
        }
        let previous_owner = self.owner.get();
        self.owner.set(new_owner);
        self.vm().log(OwnershipTransferred {
            previous_owner,
            new_owner,
        });
        Ok(())
    }

    /// Grant authorized role to a backend address. Owner only.
    pub fn grant_authorized(&mut self, account: Address) -> Result<(), ProgressError> {
        self.require_owner()?;

        if account == Address::ZERO {
            return Err(ProgressError::ZeroAddress(ZeroAddress {}));
        }

        self.authorized.setter(account).set(true);

        let granted_by = self.vm().msg_sender();
        self.vm().log(AuthorizedGranted {
            account,
            granted_by,
        });

        Ok(())
    }

    /// Revoke authorized role from an account. Owner only.
    pub fn revoke_authorized(&mut self, account: Address) -> Result<(), ProgressError> {
        self.require_owner()?;

        self.authorized.setter(account).set(false);

        let revoked_by = self.vm().msg_sender();
        self.vm().log(AuthorizedRevoked {
            account,
            revoked_by,
        });

        Ok(())
    }

    /// Update the DIUReputation contract address. Owner only.
    ///
    /// Used when DIUReputation is redeployed (Phase 2 without proxy).
    /// Pass `Address::ZERO` to temporarily disable XP calls.
    pub fn set_reputation_contract(&mut self, new_address: Address) -> Result<(), ProgressError> {
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
    pub fn get_experiment_stats(&self, user: Address, experiment_id: U256) -> (U256, U256, bool) {
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
    pub fn get_export_snapshot(&self, user: Address) -> Result<ExportSnapshot, ProgressError> {
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

    /// Get the storage layout version (ADR D-025; = 2 per ADR D-087).
    pub fn storage_version(&self) -> U256 {
        self.storage_version.get()
    }

    /// Number of distinct addresses that have attested `content_hash` (ADR D-087).
    pub fn attestation_count(&self, content_hash: FixedBytes<32>) -> U256 {
        self.attestation_count.get(content_hash)
    }

    /// Whether `attester` has attested `content_hash` (ADR D-087).
    pub fn has_attested(&self, content_hash: FixedBytes<32>, attester: Address) -> bool {
        self.attested.get(content_hash).get(attester)
    }

    /// Current meta-tx nonce for `attester` (ADR D-087, anti-replay per D-026).
    pub fn relay_nonce_of(&self, attester: Address) -> U256 {
        self.relay_nonces.get(attester)
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
    const CHARLIE: Address = address!("cccccccccccccccccccccccccccccccccccccccc");

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
        let (run_counts, best_scores, completed) = contract.get_export_snapshot(ALICE).unwrap();
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

    // ─── Group 8: Attestation — ADR D-087 (Q-160..Q-165) ─────────────

    use k256::ecdsa::SigningKey;

    const HASH_A: FixedBytes<32> = FixedBytes::new([0xaa; 32]);
    const HASH_B: FixedBytes<32> = FixedBytes::new([0xbb; 32]);

    /// Deterministic EIP-712 test environment (Arbitrum Sepolia chain id).
    const CHAIN_ID: u64 = 421614;
    const PROGRESS_ADDR: Address = address!("7777777777777777777777777777777777777777");
    const BLOCK_TS: u64 = 1_750_000_000;

    /// Initialized contract with deterministic chain id, contract address and
    /// block timestamp for EIP-712 fixtures.
    fn setup_attest() -> (TestVM, DIUProgress) {
        let (vm, contract) = setup();
        vm.set_chain_id(CHAIN_ID);
        vm.set_contract_address(PROGRESS_ADDR);
        vm.set_block_timestamp(BLOCK_TS);
        (vm, contract)
    }

    /// keccak256 of the AttestationRecorded event signature (topic 0).
    fn attestation_topic0() -> B256 {
        crypto::keccak(b"AttestationRecorded(bytes32,address,uint64,bool)")
    }

    /// All AttestationRecorded logs emitted so far: (topics, data).
    fn attestation_logs(vm: &TestVM) -> Vec<(Vec<B256>, Vec<u8>)> {
        vm.get_emitted_logs()
            .into_iter()
            .filter(|(topics, _)| topics[0] == attestation_topic0())
            .collect()
    }

    /// Deterministic secp256k1 test keypair → (signing key, EVM address).
    fn test_signer() -> (SigningKey, Address) {
        let sk = SigningKey::from_slice(&[0x42u8; 32]).unwrap();
        let pubkey = sk.verifying_key().to_encoded_point(false);
        let hash = crypto::keccak(&pubkey.as_bytes()[1..]);
        (sk, Address::from_slice(&hash.as_slice()[12..]))
    }

    /// Independent EIP-712 digest implementation mirroring the ADR D-087 spec.
    ///
    /// Deliberately NOT calling the contract's `attest_digest`: a contract-side
    /// digest bug (e.g. dropped nonce) makes the precompile calldata diverge
    /// from the mock below and the test fails with InvalidSignature.
    fn eip712_attest_digest(
        content_hash: FixedBytes<32>,
        attester: Address,
        nonce: U256,
        deadline: U256,
    ) -> B256 {
        let domain_separator = {
            let mut buf = alloc::vec::Vec::new();
            buf.extend_from_slice(
                crypto::keccak(
                    b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
                )
                .as_slice(),
            );
            buf.extend_from_slice(crypto::keccak(b"DIUProgress").as_slice());
            buf.extend_from_slice(crypto::keccak(b"2").as_slice());
            buf.extend_from_slice(&U256::from(CHAIN_ID).to_be_bytes::<32>());
            buf.extend_from_slice(B256::left_padding_from(PROGRESS_ADDR.as_slice()).as_slice());
            crypto::keccak(buf)
        };
        let struct_hash = {
            let mut buf = alloc::vec::Vec::new();
            buf.extend_from_slice(
                crypto::keccak(
                    b"Attest(bytes32 contentHash,address attester,uint256 nonce,uint256 deadline)",
                )
                .as_slice(),
            );
            buf.extend_from_slice(content_hash.as_slice());
            buf.extend_from_slice(B256::left_padding_from(attester.as_slice()).as_slice());
            buf.extend_from_slice(&nonce.to_be_bytes::<32>());
            buf.extend_from_slice(&deadline.to_be_bytes::<32>());
            crypto::keccak(buf)
        };
        let mut preimage = alloc::vec![0x19u8, 0x01u8];
        preimage.extend_from_slice(domain_separator.as_slice());
        preimage.extend_from_slice(struct_hash.as_slice());
        crypto::keccak(preimage)
    }

    /// Sign `digest`, mock the ecrecover precompile for exactly the calldata the
    /// contract must produce, and return the 65-byte `r||s||v` signature.
    fn sign_and_mock(vm: &TestVM, sk: &SigningKey, signer: Address, digest: B256) -> Bytes {
        let (sig, recid) = sk.sign_prehash_recoverable(digest.as_slice()).unwrap();
        let mut sig_bytes = [0u8; 65];
        sig_bytes[..64].copy_from_slice(&sig.to_bytes());
        sig_bytes[64] = 27 + recid.to_byte();

        let mut input = [0u8; 128];
        input[..32].copy_from_slice(digest.as_slice());
        input[63] = sig_bytes[64];
        input[64..128].copy_from_slice(&sig_bytes[..64]);
        vm.mock_static_call(
            ECRECOVER_PRECOMPILE,
            input.to_vec(),
            Ok(B256::left_padding_from(signer.as_slice()).to_vec()),
        );

        Bytes::from(sig_bytes.to_vec())
    }

    /// Mock the precompile for the calldata the contract derives from
    /// (`digest`, `sig`) so it returns `recovered`.
    ///
    /// Emulates real ecrecover semantics: a signature checked against a
    /// different digest recovers a *different* key, not the attester. Needed
    /// explicitly because TestVM replays the last mock's return buffer for
    /// unmocked calls instead of the precompile's empty output.
    fn mock_recover_as(vm: &TestVM, digest: B256, sig: &Bytes, recovered: Address) {
        let mut input = [0u8; 128];
        input[..32].copy_from_slice(digest.as_slice());
        input[63] = sig[64];
        input[64..128].copy_from_slice(&sig[..64]);
        vm.mock_static_call(
            ECRECOVER_PRECOMPILE,
            input.to_vec(),
            Ok(B256::left_padding_from(recovered.as_slice()).to_vec()),
        );
    }

    #[test]
    fn test_q160_attest_result_records_and_emits() {
        let (vm, mut contract) = setup_attest();
        vm.set_sender(ALICE);
        contract.attest_result(HASH_A).unwrap();

        assert!(contract.has_attested(HASH_A, ALICE));
        assert_eq!(contract.attestation_count(HASH_A), U256::from(1));

        let logs = attestation_logs(&vm);
        assert_eq!(logs.len(), 1);
        let (topics, data) = &logs[0];
        assert_eq!(topics[1], B256::from(HASH_A));
        assert_eq!(topics[2], B256::left_padding_from(ALICE.as_slice()));
        // data = abi.encode(uint64 timestamp, bool via_relay)
        assert_eq!(data.len(), 64);
        assert_eq!(&data[24..32], &BLOCK_TS.to_be_bytes());
        assert_eq!(data[63], 0); // via_relay = false
    }

    #[test]
    fn test_q161_attest_result_repeat_same_address_reverts() {
        let (vm, mut contract) = setup_attest();
        vm.set_sender(ALICE);
        contract.attest_result(HASH_A).unwrap();

        let result = contract.attest_result(HASH_A);
        assert!(matches!(result, Err(ProgressError::AlreadyAttested(_))));
        assert_eq!(contract.attestation_count(HASH_A), U256::from(1));
    }

    #[test]
    fn test_q162_two_addresses_count_and_hash_independence() {
        let (vm, mut contract) = setup_attest();
        vm.set_sender(ALICE);
        contract.attest_result(HASH_A).unwrap();
        vm.set_sender(BOB);
        contract.attest_result(HASH_A).unwrap();
        assert_eq!(contract.attestation_count(HASH_A), U256::from(2));

        // A different hash from the same address is independent.
        contract.attest_result(HASH_B).unwrap();
        assert_eq!(contract.attestation_count(HASH_B), U256::from(1));
        assert!(contract.has_attested(HASH_B, BOB));
        assert!(!contract.has_attested(HASH_B, ALICE));
        assert_eq!(contract.attestation_count(HASH_A), U256::from(2));
    }

    #[test]
    fn test_q163_attest_with_sig_relayed_by_other_address() {
        let (vm, mut contract) = setup_attest();
        let (sk, signer) = test_signer();
        let deadline = U256::from(BLOCK_TS + 3600);

        let digest = eip712_attest_digest(HASH_A, signer, U256::ZERO, deadline);
        let sig = sign_and_mock(&vm, &sk, signer, digest);

        vm.set_sender(BOB); // relayer pays gas, signer is the attester
        contract
            .attest_with_sig(HASH_A, signer, deadline, sig)
            .unwrap();

        assert!(contract.has_attested(HASH_A, signer));
        assert!(!contract.has_attested(HASH_A, BOB));
        assert_eq!(contract.attestation_count(HASH_A), U256::from(1));
        assert_eq!(contract.relay_nonce_of(signer), U256::from(1));

        let logs = attestation_logs(&vm);
        assert_eq!(logs.len(), 1);
        let (topics, data) = &logs[0];
        assert_eq!(topics[1], B256::from(HASH_A));
        assert_eq!(topics[2], B256::left_padding_from(signer.as_slice()));
        assert_eq!(data[63], 1); // via_relay = true
    }

    #[test]
    fn test_q164_replay_expired_and_wrong_hash_rejected() {
        let (vm, mut contract) = setup_attest();
        let (sk, signer) = test_signer();
        let deadline = U256::from(BLOCK_TS + 3600);

        let digest = eip712_attest_digest(HASH_A, signer, U256::ZERO, deadline);
        let sig = sign_and_mock(&vm, &sk, signer, digest);
        vm.set_sender(BOB);
        contract
            .attest_with_sig(HASH_A, signer, deadline, sig.clone())
            .unwrap();

        // Replay: nonce is now 1, the contract derives a different digest,
        // recovery no longer yields the attester → InvalidSignature.
        let replay_digest = eip712_attest_digest(HASH_A, signer, U256::from(1), deadline);
        mock_recover_as(&vm, replay_digest, &sig, CHARLIE);
        let result = contract.attest_with_sig(HASH_A, signer, deadline, sig.clone());
        assert!(matches!(result, Err(ProgressError::InvalidSignature(_))));
        assert_eq!(contract.attestation_count(HASH_A), U256::from(1));
        assert_eq!(contract.relay_nonce_of(signer), U256::from(1));

        // Expired deadline (checked before any signature work).
        let expired = U256::from(BLOCK_TS - 1);
        let result = contract.attest_with_sig(HASH_B, signer, expired, sig);
        assert!(matches!(result, Err(ProgressError::ExpiredSignature(_))));

        // Signature over HASH_A (current nonce) submitted for HASH_B.
        let digest = eip712_attest_digest(HASH_A, signer, U256::from(1), deadline);
        let sig = sign_and_mock(&vm, &sk, signer, digest);
        let wrong_digest = eip712_attest_digest(HASH_B, signer, U256::from(1), deadline);
        mock_recover_as(&vm, wrong_digest, &sig, CHARLIE);
        let result = contract.attest_with_sig(HASH_B, signer, deadline, sig);
        assert!(matches!(result, Err(ProgressError::InvalidSignature(_))));
        assert!(!contract.has_attested(HASH_B, signer));
    }

    #[test]
    fn test_q165_pause_blocks_attestation_unpause_restores() {
        let (vm, mut contract) = setup_attest();

        // Owner is the pause guardian (set in initialize).
        contract.pause().unwrap();
        assert!(contract.is_paused());

        vm.set_sender(ALICE);
        let result = contract.attest_result(HASH_A);
        assert!(matches!(result, Err(ProgressError::ContractPaused(_))));

        let (sk, signer) = test_signer();
        let deadline = U256::from(BLOCK_TS + 3600);
        let digest = eip712_attest_digest(HASH_A, signer, U256::ZERO, deadline);
        let sig = sign_and_mock(&vm, &sk, signer, digest);
        let result = contract.attest_with_sig(HASH_A, signer, deadline, sig.clone());
        assert!(matches!(result, Err(ProgressError::ContractPaused(_))));

        vm.set_sender(OWNER);
        contract.unpause().unwrap();
        assert!(!contract.is_paused());

        vm.set_sender(ALICE);
        contract.attest_result(HASH_A).unwrap();
        vm.set_sender(BOB);
        contract
            .attest_with_sig(HASH_A, signer, deadline, sig)
            .unwrap();
        assert_eq!(contract.attestation_count(HASH_A), U256::from(2));
    }

    // ─── Group 9: v2 admin surface (D-087 §6, D-029) ──────────────────

    #[test]
    fn test_storage_version_is_2() {
        let (_, contract) = setup();
        assert_eq!(contract.storage_version(), U256::from(2));
    }

    #[test]
    fn test_pause_non_guardian_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);
        assert!(matches!(
            contract.pause(),
            Err(ProgressError::Unauthorized(_))
        ));
    }

    #[test]
    fn test_set_pause_guardian_owner_only() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);
        assert!(matches!(
            contract.set_pause_guardian(ALICE),
            Err(ProgressError::Unauthorized(_))
        ));

        vm.set_sender(OWNER);
        contract.set_pause_guardian(BOB).unwrap();
        assert_eq!(contract.get_pause_guardian(), BOB);

        // New guardian can pause; old owner no longer can.
        assert!(matches!(
            contract.pause(),
            Err(ProgressError::Unauthorized(_))
        ));
        vm.set_sender(BOB);
        contract.pause().unwrap();
        assert!(contract.is_paused());
    }

    #[test]
    fn test_transfer_ownership_success_and_guards() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);
        assert!(matches!(
            contract.transfer_ownership(ALICE),
            Err(ProgressError::Unauthorized(_))
        ));

        vm.set_sender(OWNER);
        assert!(matches!(
            contract.transfer_ownership(Address::ZERO),
            Err(ProgressError::ZeroAddress(_))
        ));

        contract.transfer_ownership(BOB).unwrap();
        assert_eq!(contract.owner(), BOB);
        assert!(contract.is_authorized(BOB));
    }
}
