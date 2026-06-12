//! DIUReputation — XP tracking, level calculation, daily login streaks.
//!
//! Tracks user experience points (XP), computes levels, and maintains
//! daily login streaks. Only authorized backend addresses can award XP
//! and record logins. View functions are public.

use alloc::vec::Vec;
use stylus_sdk::{
    alloy_primitives::{Address, U256, U64},
    alloy_sol_types::sol,
    prelude::*,
};

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Seconds per day.
const SECONDS_PER_DAY: u64 = 86_400;

/// XP awarded for a daily login.
const DAILY_LOGIN_XP: u64 = 10;

/// Maximum XP a user can earn in a single calendar day (Gap #4 / E-4).
///
/// Covers: ~5 simulations × 50 XP + module completion 100 XP + daily login 10 XP.
/// Applies to all XP paths (add_xp + record_daily_login) via internal_add_xp.
const MAX_DAILY_XP: u64 = 500;

/// Level thresholds (checked top-down): (min_xp, level).
/// Levels: 1(0), 2(100), 3(300), 4(600), 5(1000).
const LEVEL_THRESHOLDS: [(u64, u8); 5] = [(1000, 5), (600, 4), (300, 3), (100, 2), (0, 1)];

// ═══════════════════════════════════════════════════════════════════════════
// EVENTS
// ═══════════════════════════════════════════════════════════════════════════

sol! {
    /// Emitted when the contract is initialized.
    event Initialized(address indexed owner);

    /// Emitted when XP is awarded to a user.
    event XpAwarded(address indexed user, uint256 amount, uint256 total_xp);

    /// Emitted when a user records a daily login.
    event DailyLoginRecorded(address indexed user, uint256 current_streak);

    /// Emitted when a user reaches a new level.
    event LevelUp(address indexed user, uint8 new_level);

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
    error ZeroAmount();
    error AlreadyLoggedInToday();
    error AlreadyInitialized();
    /// XP addition would overflow U256 — should never happen in practice.
    error ArithmeticOverflow();
    /// Provided nonce does not match the expected per-user nonce (ADR D-026).
    error InvalidNonce();
    /// User has reached the daily XP cap (MAX_DAILY_XP). Resets at next UTC day (Gap #4).
    error DailyXpCapExceeded();
}

/// Contract-level error type for DIUReputation.
#[derive(SolidityError)]
pub enum ReputationError {
    Unauthorized(Unauthorized),
    ZeroAddress(ZeroAddress),
    ZeroAmount(ZeroAmount),
    AlreadyLoggedInToday(AlreadyLoggedInToday),
    AlreadyInitialized(AlreadyInitialized),
    ArithmeticOverflow(ArithmeticOverflow),
    InvalidNonce(InvalidNonce),
    DailyXpCapExceeded(DailyXpCapExceeded),
}

impl core::fmt::Debug for ReputationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unauthorized(_) => write!(f, "Unauthorized"),
            Self::ZeroAddress(_) => write!(f, "ZeroAddress"),
            Self::ZeroAmount(_) => write!(f, "ZeroAmount"),
            Self::AlreadyLoggedInToday(_) => write!(f, "AlreadyLoggedInToday"),
            Self::AlreadyInitialized(_) => write!(f, "AlreadyInitialized"),
            Self::ArithmeticOverflow(_) => write!(f, "ArithmeticOverflow"),
            Self::InvalidNonce(_) => write!(f, "InvalidNonce"),
            Self::DailyXpCapExceeded(_) => write!(f, "DailyXpCapExceeded"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STORAGE
// ═══════════════════════════════════════════════════════════════════════════

sol_storage! {
    #[entrypoint]
    pub struct DIUReputation {
        /// Contract owner (set at deployment).
        address owner;

        /// Whether the contract has been initialized.
        bool initialized;

        /// Authorized backend addresses that can award XP and record logins.
        mapping(address => bool) authorized;

        /// Total XP per user.
        mapping(address => uint256) xp;

        /// Day number of last recorded login (block_timestamp / 86400).
        mapping(address => uint256) last_login_day;

        /// Current consecutive daily login streak.
        mapping(address => uint256) current_streak;

        /// Longest streak ever achieved by a user.
        mapping(address => uint256) longest_streak;

        /// List of all addresses that have received XP (for leaderboard).
        address[] users_with_xp;

        /// Whether an address is already in the users_with_xp list.
        mapping(address => bool) has_xp_entry;

        /// Total XP awarded across all users.
        uint256 total_xp_awarded;

        /// Per-user nonce for logical replay prevention (ADR D-026).
        /// Backend must pass the current nonce on every add_xp call.
        mapping(address => uint64) user_nonces;

        /// Reserved storage slots for future upgrades (ADR D-025).
        uint256 _reserved0;
        uint256 _reserved1;
        uint256 _reserved2;
        uint256 _reserved3;

        /// Day number (block_timestamp / 86400) when daily XP tracking was last reset per user.
        mapping(address => uint256) last_xp_day;

        /// XP earned by a user in the current calendar day. Resets when day changes (Gap #4).
        mapping(address => uint256) daily_xp_earned;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INTERNAL HELPERS
// ═══════════════════════════════════════════════════════════════════════════

impl DIUReputation {
    /// Returns an error if the caller is not the contract owner.
    fn require_owner(&self) -> Result<(), ReputationError> {
        if self.vm().msg_sender() != self.owner.get() {
            return Err(ReputationError::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }

    /// Returns an error if the caller is not authorized (owner counts as authorized).
    fn require_authorized(&self) -> Result<(), ReputationError> {
        let caller = self.vm().msg_sender();
        if caller != self.owner.get() && !self.authorized.get(caller) {
            return Err(ReputationError::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }

    /// Compute the level for a given XP amount.
    ///
    /// Levels: 1(0 XP), 2(100), 3(300), 4(600), 5(1000).
    fn compute_level(xp: U256) -> u8 {
        for &(threshold, level) in &LEVEL_THRESHOLDS {
            if xp >= U256::from(threshold) {
                return level;
            }
        }
        1
    }

    /// Internal: add XP to a user, track in user list, emit events.
    ///
    /// Enforces the per-user daily XP cap (`MAX_DAILY_XP`). The cap resets at
    /// UTC day boundary (`block_timestamp / 86400`). Both `add_xp` and
    /// `record_daily_login` go through this function — the cap is a single
    /// enforcement point (Gap #4).
    ///
    /// Note: if a user hits the cap mid-day before `record_daily_login`, the
    /// streak is still recorded but the 10 XP are not awarded. Acceptable
    /// edge case for testnet Phase 2.
    fn internal_add_xp(&mut self, user: Address, amount: U256) -> Result<(), ReputationError> {
        // ── Daily XP cap (Gap #4 / E-4) ──────────────────────────────
        let today = U256::from(self.vm().block_timestamp() / SECONDS_PER_DAY);
        let last = self.last_xp_day.get(user);
        let earned = if last == today {
            self.daily_xp_earned.get(user)
        } else {
            // New day (or first ever award) — reset tracking
            self.last_xp_day.setter(user).set(today);
            U256::ZERO
        };
        let new_earned = earned
            .checked_add(amount)
            .ok_or(ReputationError::ArithmeticOverflow(ArithmeticOverflow {}))?;
        if new_earned > U256::from(MAX_DAILY_XP) {
            return Err(ReputationError::DailyXpCapExceeded(DailyXpCapExceeded {}));
        }
        self.daily_xp_earned.setter(user).set(new_earned);
        // ─────────────────────────────────────────────────────────────

        let old_xp = self.xp.get(user);
        let new_xp = old_xp
            .checked_add(amount)
            .ok_or(ReputationError::ArithmeticOverflow(ArithmeticOverflow {}))?;
        self.xp.setter(user).set(new_xp);

        // Track user in the list for leaderboard enumeration
        if !self.has_xp_entry.get(user) {
            self.users_with_xp.push(user);
            self.has_xp_entry.setter(user).set(true);
        }

        // Update global total (also checked)
        let total = self.total_xp_awarded.get();
        let new_total = total
            .checked_add(amount)
            .ok_or(ReputationError::ArithmeticOverflow(ArithmeticOverflow {}))?;
        self.total_xp_awarded.set(new_total);

        self.vm().log(XpAwarded {
            user,
            amount,
            total_xp: new_xp,
        });

        // Check for level up
        let old_level = Self::compute_level(old_xp);
        let new_level = Self::compute_level(new_xp);
        if new_level > old_level {
            self.vm().log(LevelUp { user, new_level });
        }

        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC INTERFACE
// ═══════════════════════════════════════════════════════════════════════════

#[public]
impl DIUReputation {
    // ─── Initialization ──────────────────────────────────────────────

    /// Initialize the contract. Can only be called once.
    ///
    /// Sets the caller as owner and grants initial authorized role.
    pub fn initialize(&mut self) -> Result<(), ReputationError> {
        if self.initialized.get() {
            return Err(ReputationError::AlreadyInitialized(AlreadyInitialized {}));
        }

        let caller = self.vm().msg_sender();
        self.owner.set(caller);
        self.authorized.setter(caller).set(true);
        self.initialized.set(true);

        self.vm().log(Initialized { owner: caller });
        Ok(())
    }

    // ─── Write Functions (Authorized Only) ───────────────────────────

    /// Awards XP to a user with replay protection.
    ///
    /// # Nonce
    /// Caller must read the current nonce via `get_nonce(user)` before calling.
    /// Each successful call increments the nonce by 1.
    /// Passing an incorrect nonce returns `InvalidNonce`.
    ///
    /// # Example (backend flow)
    /// 1. `nonce = get_nonce(user)`
    /// 2. `add_xp(user, amount, nonce)`
    pub fn add_xp(
        &mut self,
        user: Address,
        amount: U256,
        nonce: u64,
    ) -> Result<(), ReputationError> {
        self.require_authorized()?;

        if user == Address::ZERO {
            return Err(ReputationError::ZeroAddress(ZeroAddress {}));
        }
        if amount == U256::ZERO {
            return Err(ReputationError::ZeroAmount(ZeroAmount {}));
        }

        let expected = self.user_nonces.get(user).saturating_to::<u64>();
        if nonce != expected {
            return Err(ReputationError::InvalidNonce(InvalidNonce {}));
        }

        self.internal_add_xp(user, amount)?;
        // Increment only after successful XP award so a failed internal_add_xp
        // does not consume the nonce (matches on-chain revert semantics in tests).
        self.user_nonces
            .setter(user)
            .set(U64::from(nonce.wrapping_add(1)));

        Ok(())
    }

    /// Record a daily login for a user. Awards 10 XP. Authorized callers only.
    ///
    /// Streaks increment when called on consecutive days. Missing a day
    /// resets the streak to 1.
    pub fn record_daily_login(&mut self, user: Address) -> Result<(), ReputationError> {
        self.require_authorized()?;

        if user == Address::ZERO {
            return Err(ReputationError::ZeroAddress(ZeroAddress {}));
        }

        let today = U256::from(self.vm().block_timestamp() / SECONDS_PER_DAY);
        let last = self.last_login_day.get(user);

        // Prevent double-login on the same day (skip check for first-ever login)
        if last == today && last != U256::ZERO {
            return Err(ReputationError::AlreadyLoggedInToday(
                AlreadyLoggedInToday {},
            ));
        }

        // Update streak
        let one = U256::from(1);
        let new_streak = if last + one == today {
            // Consecutive day — extend streak
            self.current_streak.get(user) + one
        } else {
            // First login or missed a day — start new streak
            one
        };

        self.current_streak.setter(user).set(new_streak);
        self.last_login_day.setter(user).set(today);

        // Update longest streak if beaten
        let longest = self.longest_streak.get(user);
        if new_streak > longest {
            self.longest_streak.setter(user).set(new_streak);
        }

        self.vm().log(DailyLoginRecorded {
            user,
            current_streak: new_streak,
        });

        // Award daily login XP
        self.internal_add_xp(user, U256::from(DAILY_LOGIN_XP))?;

        Ok(())
    }

    // ─── Access Control (Owner Only) ─────────────────────────────────

    /// Grant authorized role to an account. Owner only.
    pub fn grant_authorized(&mut self, account: Address) -> Result<(), ReputationError> {
        self.require_owner()?;

        if account == Address::ZERO {
            return Err(ReputationError::ZeroAddress(ZeroAddress {}));
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
    pub fn revoke_authorized(&mut self, account: Address) -> Result<(), ReputationError> {
        self.require_owner()?;

        self.authorized.setter(account).set(false);

        let revoked_by = self.vm().msg_sender();
        self.vm().log(AuthorizedRevoked {
            account,
            revoked_by,
        });

        Ok(())
    }

    // ─── View Functions ──────────────────────────────────────────────

    /// Get the total XP for a user.
    pub fn get_xp(&self, user: Address) -> U256 {
        self.xp.get(user)
    }

    /// Get the level for a user (1-5).
    ///
    /// Levels: 1(0 XP), 2(100), 3(300), 4(600), 5(1000).
    pub fn get_level(&self, user: Address) -> u8 {
        Self::compute_level(self.xp.get(user))
    }

    /// Get the current daily login streak for a user.
    pub fn get_streak(&self, user: Address) -> U256 {
        self.current_streak.get(user)
    }

    /// Get the longest daily login streak ever achieved by a user.
    pub fn get_longest_streak(&self, user: Address) -> U256 {
        self.longest_streak.get(user)
    }

    /// Get the top users by XP (descending). Returns parallel arrays.
    ///
    /// Suitable for off-chain reads via `eth_call` (free). For large user
    /// bases, prefer off-chain indexing via events.
    pub fn get_leaderboard(&self, limit: U256) -> (Vec<Address>, Vec<U256>) {
        let len = self.users_with_xp.len();
        let cap = core::cmp::min(limit.saturating_to::<usize>(), len);

        // Collect all (address, xp) pairs
        let mut entries: Vec<(Address, U256)> = Vec::with_capacity(len);
        for i in 0..len {
            let user = self.users_with_xp.get(i).expect("index in bounds");
            let xp = self.xp.get(user);
            entries.push((user, xp));
        }

        // Sort descending by XP
        entries.sort_by(|a, b| b.1.cmp(&a.1));
        entries.truncate(cap);

        // Split into parallel arrays for ABI compatibility
        let mut addrs = Vec::with_capacity(cap);
        let mut xps = Vec::with_capacity(cap);
        for (addr, xp) in entries {
            addrs.push(addr);
            xps.push(xp);
        }

        (addrs, xps)
    }

    /// Get the total XP awarded across all users.
    pub fn total_xp_awarded(&self) -> U256 {
        self.total_xp_awarded.get()
    }

    /// Get the number of users who have received XP.
    pub fn total_users(&self) -> U256 {
        U256::from(self.users_with_xp.len())
    }

    /// Get the contract owner address.
    pub fn owner(&self) -> Address {
        self.owner.get()
    }

    /// Check whether an address is authorized.
    pub fn is_authorized(&self, account: Address) -> bool {
        account == self.owner.get() || self.authorized.get(account)
    }

    /// Get the current replay-prevention nonce for a user (ADR D-026).
    ///
    /// Pass this value as `nonce` in the next `add_xp` call for this user.
    pub fn get_nonce(&self, user: Address) -> u64 {
        self.user_nonces.get(user).saturating_to::<u64>()
    }

    /// Get XP earned by a user in the current calendar day (Gap #4).
    ///
    /// Backend can use this to check remaining cap before calling `add_xp`.
    /// Remaining = MAX_DAILY_XP - get_daily_xp_earned(user).
    pub fn get_daily_xp_earned(&self, user: Address) -> U256 {
        let today = U256::from(self.vm().block_timestamp() / SECONDS_PER_DAY);
        if self.last_xp_day.get(user) == today {
            self.daily_xp_earned.get(user)
        } else {
            U256::ZERO
        }
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
    const CAROL: Address = address!("cccccccccccccccccccccccccccccccccccccccc");

    /// Helper: create a VM and initialized contract with OWNER as owner.
    fn setup() -> (TestVM, DIUReputation) {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIUReputation::from(&vm);
        contract.initialize().unwrap();
        (vm, contract)
    }

    /// Helper: setup with an authorized BACKEND address.
    fn setup_with_backend() -> (TestVM, DIUReputation) {
        let (vm, mut contract) = setup();
        contract.grant_authorized(BACKEND).unwrap();
        vm.set_sender(BACKEND);
        (vm, contract)
    }

    // ─── Initialization ──────────────────────────────────────────────

    #[test]
    fn test_initialize_sets_owner() {
        let (_, contract) = setup();
        assert_eq!(contract.owner(), OWNER);
    }

    #[test]
    fn test_initialize_owner_is_authorized() {
        let (_, contract) = setup();
        assert!(contract.is_authorized(OWNER));
    }

    #[test]
    fn test_initialize_starts_empty() {
        let (_, contract) = setup();
        assert_eq!(contract.total_xp_awarded(), U256::ZERO);
        assert_eq!(contract.total_users(), U256::ZERO);
    }

    #[test]
    fn test_initialize_twice_reverts() {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIUReputation::from(&vm);
        contract.initialize().unwrap();

        let result = contract.initialize();
        assert!(matches!(
            result,
            Err(ReputationError::AlreadyInitialized(_))
        ));
    }

    #[test]
    fn test_initialize_sets_initialized() {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIUReputation::from(&vm);

        contract.initialize().unwrap();

        // Verify by trying to initialize again
        let result = contract.initialize();
        assert!(matches!(
            result,
            Err(ReputationError::AlreadyInitialized(_))
        ));
    }

    // ─── add_xp ─────────────────────────────────────────────────────

    #[test]
    fn test_add_xp_success() {
        let (_, mut contract) = setup_with_backend();

        contract.add_xp(ALICE, U256::from(100), 0).unwrap();

        assert_eq!(contract.get_xp(ALICE), U256::from(100));
        assert_eq!(contract.total_xp_awarded(), U256::from(100));
        assert_eq!(contract.total_users(), U256::from(1));
    }

    #[test]
    fn test_add_xp_accumulates() {
        let (_, mut contract) = setup_with_backend();

        contract.add_xp(ALICE, U256::from(50), 0).unwrap();
        contract.add_xp(ALICE, U256::from(75), 1).unwrap();

        assert_eq!(contract.get_xp(ALICE), U256::from(125));
        assert_eq!(contract.total_xp_awarded(), U256::from(125));
        // Same user, only counted once
        assert_eq!(contract.total_users(), U256::from(1));
    }

    #[test]
    fn test_add_xp_multiple_users() {
        let (_, mut contract) = setup_with_backend();

        contract.add_xp(ALICE, U256::from(100), 0).unwrap();
        contract.add_xp(BOB, U256::from(200), 0).unwrap();

        assert_eq!(contract.get_xp(ALICE), U256::from(100));
        assert_eq!(contract.get_xp(BOB), U256::from(200));
        assert_eq!(contract.total_xp_awarded(), U256::from(300));
        assert_eq!(contract.total_users(), U256::from(2));
    }

    #[test]
    fn test_add_xp_unauthorized_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.add_xp(BOB, U256::from(100), 0);
        assert!(matches!(result, Err(ReputationError::Unauthorized(_))));
    }

    #[test]
    fn test_add_xp_zero_amount_reverts() {
        let (_, mut contract) = setup_with_backend();

        let result = contract.add_xp(ALICE, U256::ZERO, 0);
        assert!(matches!(result, Err(ReputationError::ZeroAmount(_))));
    }

    #[test]
    fn test_add_xp_overflow_reverts() {
        let (_, mut contract) = setup_with_backend();

        // Daily cap (500 XP) fires before arithmetic overflow for any large amount.
        // ArithmeticOverflow remains as defensive code but is unreachable via normal flows.
        let result = contract.add_xp(ALICE, U256::MAX, 0);
        assert!(matches!(
            result,
            Err(ReputationError::DailyXpCapExceeded(_))
        ));
        // XP must not have been awarded
        assert_eq!(contract.get_xp(ALICE), U256::ZERO);
    }

    #[test]
    fn test_add_xp_zero_address_reverts() {
        let (_, mut contract) = setup_with_backend();

        let result = contract.add_xp(Address::ZERO, U256::from(100), 0);
        assert!(matches!(result, Err(ReputationError::ZeroAddress(_))));
    }

    // ─── Levels ──────────────────────────────────────────────────────

    #[test]
    fn test_level_starts_at_1() {
        let (_, contract) = setup();
        assert_eq!(contract.get_level(ALICE), 1);
    }

    #[test]
    fn test_level_2_at_100_xp() {
        let (_, mut contract) = setup_with_backend();
        contract.add_xp(ALICE, U256::from(100), 0).unwrap();
        assert_eq!(contract.get_level(ALICE), 2);
    }

    #[test]
    fn test_level_3_at_300_xp() {
        let (_, mut contract) = setup_with_backend();
        contract.add_xp(ALICE, U256::from(300), 0).unwrap();
        assert_eq!(contract.get_level(ALICE), 3);
    }

    #[test]
    fn test_level_4_at_600_xp() {
        let (vm, mut contract) = setup_with_backend();
        // 600 XP > daily cap (500) — split across two days
        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);
        contract.add_xp(ALICE, U256::from(300), 0).unwrap();
        vm.set_block_timestamp(20001 * SECONDS_PER_DAY);
        contract.add_xp(ALICE, U256::from(300), 1).unwrap();
        assert_eq!(contract.get_level(ALICE), 4);
    }

    #[test]
    fn test_level_5_at_1000_xp() {
        let (vm, mut contract) = setup_with_backend();
        // 1000 XP > daily cap (500) — split across two days
        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);
        contract.add_xp(ALICE, U256::from(500), 0).unwrap();
        vm.set_block_timestamp(20001 * SECONDS_PER_DAY);
        contract.add_xp(ALICE, U256::from(500), 1).unwrap();
        assert_eq!(contract.get_level(ALICE), 5);
    }

    #[test]
    fn test_level_boundary_99_is_level_1() {
        let (_, mut contract) = setup_with_backend();
        contract.add_xp(ALICE, U256::from(99), 0).unwrap();
        assert_eq!(contract.get_level(ALICE), 1);
    }

    #[test]
    fn test_level_above_1000() {
        let (vm, mut contract) = setup_with_backend();
        // 5000 XP > daily cap (500) — award 500/day across 10 days
        for day in 0u64..10 {
            vm.set_block_timestamp((20000 + day) * SECONDS_PER_DAY);
            contract.add_xp(ALICE, U256::from(500), day).unwrap();
        }
        assert_eq!(contract.get_level(ALICE), 5);
    }

    // ─── record_daily_login ──────────────────────────────────────────

    #[test]
    fn test_daily_login_first_time() {
        let (vm, mut contract) = setup_with_backend();
        // Day 20000 (~2024)
        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);

        contract.record_daily_login(ALICE).unwrap();

        assert_eq!(contract.get_streak(ALICE), U256::from(1));
        assert_eq!(contract.get_longest_streak(ALICE), U256::from(1));
        assert_eq!(contract.get_xp(ALICE), U256::from(DAILY_LOGIN_XP));
    }

    #[test]
    fn test_daily_login_consecutive_days() {
        let (vm, mut contract) = setup_with_backend();

        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);
        contract.record_daily_login(ALICE).unwrap();

        vm.set_block_timestamp(20001 * SECONDS_PER_DAY);
        contract.record_daily_login(ALICE).unwrap();

        vm.set_block_timestamp(20002 * SECONDS_PER_DAY);
        contract.record_daily_login(ALICE).unwrap();

        assert_eq!(contract.get_streak(ALICE), U256::from(3));
        assert_eq!(contract.get_longest_streak(ALICE), U256::from(3));
        assert_eq!(contract.get_xp(ALICE), U256::from(DAILY_LOGIN_XP * 3));
    }

    #[test]
    fn test_daily_login_streak_resets_on_gap() {
        let (vm, mut contract) = setup_with_backend();

        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);
        contract.record_daily_login(ALICE).unwrap();

        vm.set_block_timestamp(20001 * SECONDS_PER_DAY);
        contract.record_daily_login(ALICE).unwrap();

        // Skip day 20002
        vm.set_block_timestamp(20003 * SECONDS_PER_DAY);
        contract.record_daily_login(ALICE).unwrap();

        assert_eq!(contract.get_streak(ALICE), U256::from(1));
        // Longest streak preserved
        assert_eq!(contract.get_longest_streak(ALICE), U256::from(2));
    }

    #[test]
    fn test_daily_login_same_day_reverts() {
        let (vm, mut contract) = setup_with_backend();

        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);
        contract.record_daily_login(ALICE).unwrap();

        let result = contract.record_daily_login(ALICE);
        assert!(matches!(
            result,
            Err(ReputationError::AlreadyLoggedInToday(_))
        ));
    }

    #[test]
    fn test_daily_login_unauthorized_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);
        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);

        let result = contract.record_daily_login(BOB);
        assert!(matches!(result, Err(ReputationError::Unauthorized(_))));
    }

    #[test]
    fn test_daily_login_zero_address_reverts() {
        let (vm, mut contract) = setup_with_backend();
        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);

        let result = contract.record_daily_login(Address::ZERO);
        assert!(matches!(result, Err(ReputationError::ZeroAddress(_))));
    }

    // ─── Leaderboard ─────────────────────────────────────────────────

    #[test]
    fn test_leaderboard_empty() {
        let (_, contract) = setup();

        let (addrs, xps) = contract.get_leaderboard(U256::from(10));
        assert!(addrs.is_empty());
        assert!(xps.is_empty());
    }

    #[test]
    fn test_leaderboard_sorted_descending() {
        let (_, mut contract) = setup_with_backend();

        contract.add_xp(ALICE, U256::from(100), 0).unwrap();
        contract.add_xp(BOB, U256::from(300), 0).unwrap();
        contract.add_xp(CAROL, U256::from(200), 0).unwrap();

        let (addrs, xps) = contract.get_leaderboard(U256::from(10));

        assert_eq!(addrs.len(), 3);
        assert_eq!(addrs[0], BOB);
        assert_eq!(xps[0], U256::from(300));
        assert_eq!(addrs[1], CAROL);
        assert_eq!(xps[1], U256::from(200));
        assert_eq!(addrs[2], ALICE);
        assert_eq!(xps[2], U256::from(100));
    }

    #[test]
    fn test_leaderboard_respects_limit() {
        let (_, mut contract) = setup_with_backend();

        contract.add_xp(ALICE, U256::from(100), 0).unwrap();
        contract.add_xp(BOB, U256::from(300), 0).unwrap();
        contract.add_xp(CAROL, U256::from(200), 0).unwrap();

        let (addrs, xps) = contract.get_leaderboard(U256::from(2));

        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0], BOB);
        assert_eq!(xps[0], U256::from(300));
        assert_eq!(addrs[1], CAROL);
        assert_eq!(xps[1], U256::from(200));
    }

    // ─── Access Control ──────────────────────────────────────────────

    #[test]
    fn test_grant_authorized_success() {
        let (_, mut contract) = setup();

        assert!(!contract.is_authorized(BACKEND));
        contract.grant_authorized(BACKEND).unwrap();
        assert!(contract.is_authorized(BACKEND));
    }

    #[test]
    fn test_grant_authorized_not_owner_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.grant_authorized(BACKEND);
        assert!(matches!(result, Err(ReputationError::Unauthorized(_))));
    }

    #[test]
    fn test_grant_authorized_zero_address_reverts() {
        let (_, mut contract) = setup();

        let result = contract.grant_authorized(Address::ZERO);
        assert!(matches!(result, Err(ReputationError::ZeroAddress(_))));
    }

    #[test]
    fn test_revoke_authorized_success() {
        let (_, mut contract) = setup();

        contract.grant_authorized(BACKEND).unwrap();
        assert!(contract.is_authorized(BACKEND));

        contract.revoke_authorized(BACKEND).unwrap();
        assert!(!contract.is_authorized(BACKEND));
    }

    #[test]
    fn test_revoked_cannot_add_xp() {
        let (vm, mut contract) = setup();

        contract.grant_authorized(BACKEND).unwrap();
        vm.set_sender(BACKEND);
        contract.add_xp(ALICE, U256::from(50), 0).unwrap();

        vm.set_sender(OWNER);
        contract.revoke_authorized(BACKEND).unwrap();

        vm.set_sender(BACKEND);
        // nonce=1 would be correct, but auth check fires first — any nonce value fails here
        let result = contract.add_xp(ALICE, U256::from(50), 1);
        assert!(matches!(result, Err(ReputationError::Unauthorized(_))));
    }

    // ─── Nonce (ADR D-026) ───────────────────────────────────────────

    #[test]
    fn test_get_nonce_default_zero() {
        let (_, contract) = setup();
        assert_eq!(contract.get_nonce(ALICE), 0);
    }

    #[test]
    fn test_add_xp_valid_nonce_zero() {
        let (_, mut contract) = setup_with_backend();
        contract.add_xp(ALICE, U256::from(100), 0).unwrap();
        assert_eq!(contract.get_xp(ALICE), U256::from(100));
    }

    #[test]
    fn test_add_xp_nonce_increments_after_success() {
        let (_, mut contract) = setup_with_backend();
        assert_eq!(contract.get_nonce(ALICE), 0);
        contract.add_xp(ALICE, U256::from(50), 0).unwrap();
        assert_eq!(contract.get_nonce(ALICE), 1);
        contract.add_xp(ALICE, U256::from(50), 1).unwrap();
        assert_eq!(contract.get_nonce(ALICE), 2);
    }

    #[test]
    fn test_add_xp_invalid_nonce_reverts() {
        let (_, mut contract) = setup_with_backend();
        // Expected nonce is 0, but caller passes 1
        let result = contract.add_xp(ALICE, U256::from(100), 1);
        assert!(matches!(result, Err(ReputationError::InvalidNonce(_))));
        // XP must not have been awarded
        assert_eq!(contract.get_xp(ALICE), U256::ZERO);
    }

    #[test]
    fn test_add_xp_replay_reverts() {
        let (_, mut contract) = setup_with_backend();
        contract.add_xp(ALICE, U256::from(100), 0).unwrap();
        // Replay: same nonce=0 must be rejected
        let result = contract.add_xp(ALICE, U256::from(100), 0);
        assert!(matches!(result, Err(ReputationError::InvalidNonce(_))));
        // XP awarded only once
        assert_eq!(contract.get_xp(ALICE), U256::from(100));
    }

    #[test]
    fn test_add_xp_nonce_independent_per_user() {
        let (_, mut contract) = setup_with_backend();
        // Each user starts at nonce=0 independently
        contract.add_xp(ALICE, U256::from(50), 0).unwrap();
        contract.add_xp(BOB, U256::from(50), 0).unwrap();
        assert_eq!(contract.get_nonce(ALICE), 1);
        assert_eq!(contract.get_nonce(BOB), 1);
        // ALICE at nonce=1, BOB at nonce=1
        contract.add_xp(ALICE, U256::from(25), 1).unwrap();
        assert_eq!(contract.get_nonce(ALICE), 2);
        assert_eq!(contract.get_nonce(BOB), 1); // BOB unaffected
    }

    // ─── Daily XP Cap (Gap #4 / E-4) ────────────────────────────────

    #[test]
    fn test_daily_cap_exact_limit() {
        let (vm, mut contract) = setup_with_backend();
        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);
        // Exactly at the cap — must succeed
        contract.add_xp(ALICE, U256::from(500), 0).unwrap();
        assert_eq!(contract.get_xp(ALICE), U256::from(500));
    }

    #[test]
    fn test_daily_cap_blocks_excess() {
        let (vm, mut contract) = setup_with_backend();
        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);
        contract.add_xp(ALICE, U256::from(500), 0).unwrap();
        // One more XP must be rejected
        let result = contract.add_xp(ALICE, U256::from(1), 1);
        assert!(matches!(
            result,
            Err(ReputationError::DailyXpCapExceeded(_))
        ));
        // XP not changed
        assert_eq!(contract.get_xp(ALICE), U256::from(500));
    }

    #[test]
    fn test_daily_cap_accumulates_within_day() {
        let (vm, mut contract) = setup_with_backend();
        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);
        // Two calls: 300 + 200 = 500 — both succeed
        contract.add_xp(ALICE, U256::from(300), 0).unwrap();
        contract.add_xp(ALICE, U256::from(200), 1).unwrap();
        assert_eq!(contract.get_xp(ALICE), U256::from(500));
        // 301st XP in same day — rejected
        let result = contract.add_xp(ALICE, U256::from(1), 2);
        assert!(matches!(
            result,
            Err(ReputationError::DailyXpCapExceeded(_))
        ));
    }

    #[test]
    fn test_daily_cap_resets_next_day() {
        let (vm, mut contract) = setup_with_backend();
        // Day 1: fill cap
        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);
        contract.add_xp(ALICE, U256::from(500), 0).unwrap();
        // Day 2: cap resets — full 500 available again
        vm.set_block_timestamp(20001 * SECONDS_PER_DAY);
        contract.add_xp(ALICE, U256::from(500), 1).unwrap();
        assert_eq!(contract.get_xp(ALICE), U256::from(1000));
    }

    #[test]
    fn test_daily_cap_login_counts_toward_cap() {
        let (vm, mut contract) = setup_with_backend();
        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);
        // Login awards 10 XP via internal_add_xp — counts toward cap
        contract.record_daily_login(ALICE).unwrap();
        // 490 more XP fits within cap (10 + 490 = 500)
        contract.add_xp(ALICE, U256::from(490), 0).unwrap();
        assert_eq!(contract.get_xp(ALICE), U256::from(500));
        // One more pushes over the cap
        let result = contract.add_xp(ALICE, U256::from(1), 1);
        assert!(matches!(
            result,
            Err(ReputationError::DailyXpCapExceeded(_))
        ));
    }

    #[test]
    fn test_get_daily_xp_earned() {
        let (vm, mut contract) = setup_with_backend();
        vm.set_block_timestamp(20000 * SECONDS_PER_DAY);
        assert_eq!(contract.get_daily_xp_earned(ALICE), U256::ZERO);
        contract.add_xp(ALICE, U256::from(150), 0).unwrap();
        assert_eq!(contract.get_daily_xp_earned(ALICE), U256::from(150));
        contract.add_xp(ALICE, U256::from(100), 1).unwrap();
        assert_eq!(contract.get_daily_xp_earned(ALICE), U256::from(250));
        // Next day — view resets
        vm.set_block_timestamp(20001 * SECONDS_PER_DAY);
        assert_eq!(contract.get_daily_xp_earned(ALICE), U256::ZERO);
    }

    // ─── View Functions ──────────────────────────────────────────────

    #[test]
    fn test_get_xp_default_zero() {
        let (_, contract) = setup();
        assert_eq!(contract.get_xp(ALICE), U256::ZERO);
    }

    #[test]
    fn test_get_streak_default_zero() {
        let (_, contract) = setup();
        assert_eq!(contract.get_streak(ALICE), U256::ZERO);
    }

    #[test]
    fn test_get_longest_streak_default_zero() {
        let (_, contract) = setup();
        assert_eq!(contract.get_longest_streak(ALICE), U256::ZERO);
    }

    #[test]
    fn test_non_authorized_not_authorized() {
        let (_, contract) = setup();
        assert!(!contract.is_authorized(ALICE));
    }
}
