//! DIUToken — ERC-20 platform token for DIU OS.
//!
//! Rewards token with restricted minting (authorized backend/cross-contract),
//! public burning, admin-only pause, and standard ERC-20 transfer/approve.
//! Designed for cross-contract interaction: DIUReputation → DIUToken.mint.

use alloc::string::String;
use stylus_sdk::{
    alloy_primitives::{Address, U256},
    alloy_sol_types::sol,
    prelude::*,
};

use crate::pause::{ContractNotPaused, ContractPaused, Paused, PauseError, PauseStorage, Unpaused};

// ═══════════════════════════════════════════════════════════════════════════
// CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Token decimals (standard 18).
const DECIMALS: u8 = 18;

// ═══════════════════════════════════════════════════════════════════════════
// EVENTS
// ═══════════════════════════════════════════════════════════════════════════

sol! {
    /// Emitted when the contract is initialized.
    event Initialized(address indexed owner);

    /// ERC-20 standard Transfer event (from=0x0 on mint, to=0x0 on burn).
    event Transfer(address indexed from, address indexed to, uint256 value);

    /// ERC-20 standard Approval event.
    event Approval(address indexed owner, address indexed spender, uint256 value);

    /// Emitted when the owner grants admin role.
    event AdminGranted(address indexed account, address indexed granted_by);

    /// Emitted when the owner revokes admin role.
    event AdminRevoked(address indexed account, address indexed revoked_by);

    /// Emitted when the owner grants authorized (minter) role.
    event AuthorizedGranted(address indexed account, address indexed granted_by);

    /// Emitted when the owner revokes authorized (minter) role.
    event AuthorizedRevoked(address indexed account, address indexed revoked_by);
}

// ═══════════════════════════════════════════════════════════════════════════
// ERRORS
// ═══════════════════════════════════════════════════════════════════════════

sol! {
    error Unauthorized();
    error ZeroAddress();
    error ZeroAmount();
    error InsufficientBalance();
    error InsufficientAllowance();
    error AlreadyInitialized();
}

/// Contract-level error type for DIUToken.
#[derive(SolidityError)]
pub enum TokenError {
    Unauthorized(Unauthorized),
    ZeroAddress(ZeroAddress),
    ZeroAmount(ZeroAmount),
    InsufficientBalance(InsufficientBalance),
    InsufficientAllowance(InsufficientAllowance),
    ContractPaused(ContractPaused),
    ContractNotPaused(ContractNotPaused),
    AlreadyInitialized(AlreadyInitialized),
}

impl core::fmt::Debug for TokenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unauthorized(_) => write!(f, "Unauthorized"),
            Self::ZeroAddress(_) => write!(f, "ZeroAddress"),
            Self::ZeroAmount(_) => write!(f, "ZeroAmount"),
            Self::InsufficientBalance(_) => write!(f, "InsufficientBalance"),
            Self::InsufficientAllowance(_) => write!(f, "InsufficientAllowance"),
            Self::ContractPaused(_) => write!(f, "ContractPaused"),
            Self::ContractNotPaused(_) => write!(f, "ContractNotPaused"),
            Self::AlreadyInitialized(_) => write!(f, "AlreadyInitialized"),
        }
    }
}

impl From<PauseError> for TokenError {
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
    pub struct DIUToken {
        /// Contract owner (set at deployment).
        address owner;

        /// Whether the contract has been initialized.
        bool initialized;

        /// Addresses with admin privileges (can pause/unpause).
        mapping(address => bool) admins;

        /// Authorized addresses that can mint tokens (backend, cross-contract).
        mapping(address => bool) authorized;

        // ─── ERC-20 core ─────────────────────────────────────────────

        /// Token balances per address.
        mapping(address => uint256) balances;

        /// Spending allowances: owner → spender → amount.
        mapping(address => mapping(address => uint256)) allowances;

        /// Total token supply.
        uint256 total_supply;

        // ─── Pause state (ADR D-029) ─────────────────────────────────

        /// Pause state: paused flag + guardian address. Added at storage END per D-025.
        PauseStorage pause_state;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INTERNAL HELPERS
// ═══════════════════════════════════════════════════════════════════════════

impl DIUToken {
    /// Returns an error if the caller is not the contract owner.
    fn require_owner(&self) -> Result<(), TokenError> {
        if self.vm().msg_sender() != self.owner.get() {
            return Err(TokenError::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }

    /// Returns an error if the caller is not an admin (owner counts as admin).
    fn require_admin(&self) -> Result<(), TokenError> {
        let caller = self.vm().msg_sender();
        if caller != self.owner.get() && !self.admins.get(caller) {
            return Err(TokenError::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }

    /// Returns an error if the caller is not authorized (owner counts as authorized).
    fn require_authorized(&self) -> Result<(), TokenError> {
        let caller = self.vm().msg_sender();
        if caller != self.owner.get() && !self.authorized.get(caller) {
            return Err(TokenError::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }

    /// Internal transfer logic shared by `transfer` and `transfer_from`.
    fn internal_transfer(
        &mut self,
        from: Address,
        to: Address,
        amount: U256,
    ) -> Result<(), TokenError> {
        if to == Address::ZERO {
            return Err(TokenError::ZeroAddress(ZeroAddress {}));
        }

        let from_balance = self.balances.get(from);
        if from_balance < amount {
            return Err(TokenError::InsufficientBalance(InsufficientBalance {}));
        }

        self.balances.setter(from).set(from_balance - amount);

        let to_balance = self.balances.get(to);
        self.balances.setter(to).set(to_balance + amount);

        self.vm().log(Transfer {
            from,
            to,
            value: amount,
        });

        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC INTERFACE
// ═══════════════════════════════════════════════════════════════════════════

#[public]
impl DIUToken {
    // ─── Initialization ──────────────────────────────────────────────

    /// Initialize the contract. Can only be called once.
    ///
    /// Sets the caller as owner and grants initial admin and authorized roles.
    pub fn initialize(&mut self) -> Result<(), TokenError> {
        if self.initialized.get() {
            return Err(TokenError::AlreadyInitialized(AlreadyInitialized {}));
        }

        let caller = self.vm().msg_sender();
        self.owner.set(caller);
        self.admins.setter(caller).set(true);
        self.authorized.setter(caller).set(true);
        self.initialized.set(true);
        self.pause_state.set_pause_guardian(caller);

        self.vm().log(Initialized { owner: caller });
        Ok(())
    }

    // ─── ERC-20 Write Functions ──────────────────────────────────────

    /// Transfer tokens to another address.
    pub fn transfer(&mut self, to: Address, amount: U256) -> Result<bool, TokenError> {
        self.pause_state.require_not_paused()?;

        let caller = self.vm().msg_sender();
        self.internal_transfer(caller, to, amount)?;

        Ok(true)
    }

    /// Approve a spender to transfer tokens on behalf of the caller.
    pub fn approve(&mut self, spender: Address, amount: U256) -> Result<bool, TokenError> {
        self.pause_state.require_not_paused()?;

        if spender == Address::ZERO {
            return Err(TokenError::ZeroAddress(ZeroAddress {}));
        }

        let caller = self.vm().msg_sender();
        self.allowances.setter(caller).setter(spender).set(amount);

        self.vm().log(Approval {
            owner: caller,
            spender,
            value: amount,
        });

        Ok(true)
    }

    /// Transfer tokens from one address to another using an allowance.
    ///
    /// Infinite allowance (U256::MAX) is not decreased.
    pub fn transfer_from(
        &mut self,
        from: Address,
        to: Address,
        amount: U256,
    ) -> Result<bool, TokenError> {
        self.pause_state.require_not_paused()?;

        let caller = self.vm().msg_sender();
        let current_allowance = self.allowances.getter(from).get(caller);

        if current_allowance < amount {
            return Err(TokenError::InsufficientAllowance(
                InsufficientAllowance {},
            ));
        }

        // Don't decrease infinite allowance
        if current_allowance != U256::MAX {
            self.allowances
                .setter(from)
                .setter(caller)
                .set(current_allowance - amount);
        }

        self.internal_transfer(from, to, amount)?;

        Ok(true)
    }

    // ─── Mint & Burn ─────────────────────────────────────────────────

    /// Mint new tokens to an address. Authorized callers only.
    pub fn mint(&mut self, to: Address, amount: U256) -> Result<(), TokenError> {
        self.require_authorized()?;
        self.pause_state.require_not_paused()?;

        if to == Address::ZERO {
            return Err(TokenError::ZeroAddress(ZeroAddress {}));
        }
        if amount == U256::ZERO {
            return Err(TokenError::ZeroAmount(ZeroAmount {}));
        }

        let balance = self.balances.get(to);
        self.balances.setter(to).set(balance + amount);

        let supply = self.total_supply.get();
        self.total_supply.set(supply + amount);

        self.vm().log(Transfer {
            from: Address::ZERO,
            to,
            value: amount,
        });

        Ok(())
    }

    /// Burn tokens from the caller's balance.
    pub fn burn(&mut self, amount: U256) -> Result<(), TokenError> {
        self.pause_state.require_not_paused()?;

        if amount == U256::ZERO {
            return Err(TokenError::ZeroAmount(ZeroAmount {}));
        }

        let caller = self.vm().msg_sender();
        let balance = self.balances.get(caller);

        if balance < amount {
            return Err(TokenError::InsufficientBalance(InsufficientBalance {}));
        }

        self.balances.setter(caller).set(balance - amount);

        let supply = self.total_supply.get();
        self.total_supply.set(supply - amount);

        self.vm().log(Transfer {
            from: caller,
            to: Address::ZERO,
            value: amount,
        });

        Ok(())
    }

    // ─── Pause (Admin Only) ──────────────────────────────────────────

    /// Pause all token transfers, mints, and burns. Admin only.
    pub fn pause(&mut self) -> Result<(), TokenError> {
        self.require_admin()?;
        self.pause_state.internal_pause()?;
        let caller = self.vm().msg_sender();
        self.vm().log(Paused { guardian: caller });
        Ok(())
    }

    /// Unpause the contract. Admin only.
    pub fn unpause(&mut self) -> Result<(), TokenError> {
        self.require_admin()?;
        self.pause_state.internal_unpause()?;
        let caller = self.vm().msg_sender();
        self.vm().log(Unpaused { guardian: caller });
        Ok(())
    }

    // ─── Access Control (Owner Only) ─────────────────────────────────

    /// Grant admin role to an account. Owner only.
    pub fn grant_admin(&mut self, account: Address) -> Result<(), TokenError> {
        self.require_owner()?;

        if account == Address::ZERO {
            return Err(TokenError::ZeroAddress(ZeroAddress {}));
        }

        self.admins.setter(account).set(true);

        let granted_by = self.vm().msg_sender();
        self.vm().log(AdminGranted {
            account,
            granted_by,
        });

        Ok(())
    }

    /// Revoke admin role from an account. Owner only.
    pub fn revoke_admin(&mut self, account: Address) -> Result<(), TokenError> {
        self.require_owner()?;

        self.admins.setter(account).set(false);

        let revoked_by = self.vm().msg_sender();
        self.vm().log(AdminRevoked {
            account,
            revoked_by,
        });

        Ok(())
    }

    /// Grant authorized (minter) role to an account. Owner only.
    pub fn grant_authorized(&mut self, account: Address) -> Result<(), TokenError> {
        self.require_owner()?;

        if account == Address::ZERO {
            return Err(TokenError::ZeroAddress(ZeroAddress {}));
        }

        self.authorized.setter(account).set(true);

        let granted_by = self.vm().msg_sender();
        self.vm().log(AuthorizedGranted {
            account,
            granted_by,
        });

        Ok(())
    }

    /// Revoke authorized (minter) role from an account. Owner only.
    pub fn revoke_authorized(&mut self, account: Address) -> Result<(), TokenError> {
        self.require_owner()?;

        self.authorized.setter(account).set(false);

        let revoked_by = self.vm().msg_sender();
        self.vm().log(AuthorizedRevoked {
            account,
            revoked_by,
        });

        Ok(())
    }

    // ─── ERC-20 View Functions ───────────────────────────────────────

    /// Get the token balance of an address.
    pub fn balance_of(&self, account: Address) -> U256 {
        self.balances.get(account)
    }

    /// Get the spending allowance granted by `owner` to `spender`.
    pub fn allowance(&self, owner: Address, spender: Address) -> U256 {
        self.allowances.getter(owner).get(spender)
    }

    /// Get the total token supply.
    pub fn total_supply(&self) -> U256 {
        self.total_supply.get()
    }

    /// Token name.
    pub fn name(&self) -> String {
        String::from("DIU Token")
    }

    /// Token symbol.
    pub fn symbol(&self) -> String {
        String::from("DIU")
    }

    /// Token decimals (18).
    pub fn decimals(&self) -> u8 {
        DECIMALS
    }

    // ─── Contract View Functions ─────────────────────────────────────

    /// Get the contract owner address.
    pub fn owner(&self) -> Address {
        self.owner.get()
    }

    /// Check whether an address has admin privileges.
    pub fn is_admin(&self, account: Address) -> bool {
        account == self.owner.get() || self.admins.get(account)
    }

    /// Check whether an address is authorized to mint.
    pub fn is_authorized(&self, account: Address) -> bool {
        account == self.owner.get() || self.authorized.get(account)
    }

    /// Set the pause guardian address. Owner only. See ADR D-029.
    pub fn set_pause_guardian(&mut self, guardian: Address) -> Result<(), TokenError> {
        self.require_owner()?;
        if guardian == Address::ZERO {
            return Err(TokenError::ZeroAddress(ZeroAddress {}));
        }
        self.pause_state.set_pause_guardian(guardian);
        Ok(())
    }

    /// Get the current pause guardian address.
    pub fn get_pause_guardian(&self) -> Address {
        self.pause_state.get_pause_guardian()
    }

    /// Check whether the contract is paused.
    pub fn is_paused(&self) -> bool {
        self.pause_state.is_paused()
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
    const ADMIN: Address = address!("2222222222222222222222222222222222222222");
    const MINTER: Address = address!("3333333333333333333333333333333333333333");
    const ALICE: Address = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    const BOB: Address = address!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

    /// Helper: create a VM and initialized contract with OWNER as owner.
    fn setup() -> (TestVM, DIUToken) {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIUToken::from(&vm);
        contract.initialize().unwrap();
        (vm, contract)
    }

    /// Helper: setup and mint `amount` tokens to `to`.
    fn setup_with_tokens(to: Address, amount: U256) -> (TestVM, DIUToken) {
        let (vm, mut contract) = setup();
        contract.mint(to, amount).unwrap();
        (vm, contract)
    }

    // ─── Initialization ──────────────────────────────────────────────

    #[test]
    fn test_initialize_sets_owner() {
        let (_, contract) = setup();
        assert_eq!(contract.owner(), OWNER);
    }

    #[test]
    fn test_initialize_owner_is_admin() {
        let (_, contract) = setup();
        assert!(contract.is_admin(OWNER));
    }

    #[test]
    fn test_initialize_owner_is_authorized() {
        let (_, contract) = setup();
        assert!(contract.is_authorized(OWNER));
    }

    #[test]
    fn test_initialize_starts_unpaused() {
        let (_, contract) = setup();
        assert!(!contract.is_paused());
    }

    #[test]
    fn test_initialize_zero_supply() {
        let (_, contract) = setup();
        assert_eq!(contract.total_supply(), U256::ZERO);
    }

    #[test]
    fn test_metadata() {
        let (_, contract) = setup();
        assert_eq!(contract.name(), "DIU Token");
        assert_eq!(contract.symbol(), "DIU");
        assert_eq!(contract.decimals(), 18);
    }

    #[test]
    fn test_initialize_twice_reverts() {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIUToken::from(&vm);
        contract.initialize().unwrap();

        let result = contract.initialize();
        assert!(matches!(result, Err(TokenError::AlreadyInitialized(_))));
    }

    #[test]
    fn test_initialize_sets_initialized() {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIUToken::from(&vm);

        contract.initialize().unwrap();

        // Verify by trying to initialize again
        let result = contract.initialize();
        assert!(matches!(result, Err(TokenError::AlreadyInitialized(_))));
    }

    // ─── mint ────────────────────────────────────────────────────────

    #[test]
    fn test_mint_success() {
        let (_, mut contract) = setup();

        contract.mint(ALICE, U256::from(1000)).unwrap();

        assert_eq!(contract.balance_of(ALICE), U256::from(1000));
        assert_eq!(contract.total_supply(), U256::from(1000));
    }

    #[test]
    fn test_mint_multiple() {
        let (_, mut contract) = setup();

        contract.mint(ALICE, U256::from(500)).unwrap();
        contract.mint(BOB, U256::from(300)).unwrap();
        contract.mint(ALICE, U256::from(200)).unwrap();

        assert_eq!(contract.balance_of(ALICE), U256::from(700));
        assert_eq!(contract.balance_of(BOB), U256::from(300));
        assert_eq!(contract.total_supply(), U256::from(1000));
    }

    #[test]
    fn test_mint_by_authorized() {
        let (vm, mut contract) = setup();
        contract.grant_authorized(MINTER).unwrap();

        vm.set_sender(MINTER);
        contract.mint(ALICE, U256::from(100)).unwrap();

        assert_eq!(contract.balance_of(ALICE), U256::from(100));
    }

    #[test]
    fn test_mint_unauthorized_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.mint(BOB, U256::from(100));
        assert!(matches!(result, Err(TokenError::Unauthorized(_))));
    }

    #[test]
    fn test_mint_zero_address_reverts() {
        let (_, mut contract) = setup();

        let result = contract.mint(Address::ZERO, U256::from(100));
        assert!(matches!(result, Err(TokenError::ZeroAddress(_))));
    }

    #[test]
    fn test_mint_zero_amount_reverts() {
        let (_, mut contract) = setup();

        let result = contract.mint(ALICE, U256::ZERO);
        assert!(matches!(result, Err(TokenError::ZeroAmount(_))));
    }

    #[test]
    fn test_mint_when_paused_reverts() {
        let (_, mut contract) = setup();
        contract.pause().unwrap();

        let result = contract.mint(ALICE, U256::from(100));
        assert!(matches!(result, Err(TokenError::ContractPaused(_))));
    }

    // ─── transfer ────────────────────────────────────────────────────

    #[test]
    fn test_transfer_success() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(1000));
        vm.set_sender(ALICE);

        let ok = contract.transfer(BOB, U256::from(400)).unwrap();

        assert!(ok);
        assert_eq!(contract.balance_of(ALICE), U256::from(600));
        assert_eq!(contract.balance_of(BOB), U256::from(400));
        assert_eq!(contract.total_supply(), U256::from(1000));
    }

    #[test]
    fn test_transfer_insufficient_balance_reverts() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(100));
        vm.set_sender(ALICE);

        let result = contract.transfer(BOB, U256::from(200));
        assert!(matches!(result, Err(TokenError::InsufficientBalance(_))));
    }

    #[test]
    fn test_transfer_zero_address_reverts() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(100));
        vm.set_sender(ALICE);

        let result = contract.transfer(Address::ZERO, U256::from(50));
        assert!(matches!(result, Err(TokenError::ZeroAddress(_))));
    }

    #[test]
    fn test_transfer_when_paused_reverts() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(100));
        contract.pause().unwrap();

        vm.set_sender(ALICE);
        let result = contract.transfer(BOB, U256::from(50));
        assert!(matches!(result, Err(TokenError::ContractPaused(_))));
    }

    // ─── approve / allowance ─────────────────────────────────────────

    #[test]
    fn test_approve_success() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let ok = contract.approve(BOB, U256::from(500)).unwrap();

        assert!(ok);
        assert_eq!(contract.allowance(ALICE, BOB), U256::from(500));
    }

    #[test]
    fn test_approve_overwrite() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        contract.approve(BOB, U256::from(500)).unwrap();
        contract.approve(BOB, U256::from(200)).unwrap();

        assert_eq!(contract.allowance(ALICE, BOB), U256::from(200));
    }

    #[test]
    fn test_approve_zero_spender_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.approve(Address::ZERO, U256::from(100));
        assert!(matches!(result, Err(TokenError::ZeroAddress(_))));
    }

    #[test]
    fn test_approve_when_paused_reverts() {
        let (_, mut contract) = setup();
        contract.pause().unwrap();

        let result = contract.approve(BOB, U256::from(100));
        assert!(matches!(result, Err(TokenError::ContractPaused(_))));
    }

    // ─── transfer_from ───────────────────────────────────────────────

    #[test]
    fn test_transfer_from_success() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(1000));

        vm.set_sender(ALICE);
        contract.approve(BOB, U256::from(500)).unwrap();

        vm.set_sender(BOB);
        let ok = contract
            .transfer_from(ALICE, OWNER, U256::from(300))
            .unwrap();

        assert!(ok);
        assert_eq!(contract.balance_of(ALICE), U256::from(700));
        assert_eq!(contract.balance_of(OWNER), U256::from(300));
        assert_eq!(contract.allowance(ALICE, BOB), U256::from(200));
    }

    #[test]
    fn test_transfer_from_insufficient_allowance_reverts() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(1000));

        vm.set_sender(ALICE);
        contract.approve(BOB, U256::from(100)).unwrap();

        vm.set_sender(BOB);
        let result = contract.transfer_from(ALICE, OWNER, U256::from(200));
        assert!(matches!(
            result,
            Err(TokenError::InsufficientAllowance(_))
        ));
    }

    #[test]
    fn test_transfer_from_insufficient_balance_reverts() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(100));

        vm.set_sender(ALICE);
        contract.approve(BOB, U256::from(500)).unwrap();

        vm.set_sender(BOB);
        let result = contract.transfer_from(ALICE, OWNER, U256::from(200));
        assert!(matches!(result, Err(TokenError::InsufficientBalance(_))));
    }

    #[test]
    fn test_transfer_from_infinite_allowance() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(1000));

        vm.set_sender(ALICE);
        contract.approve(BOB, U256::MAX).unwrap();

        vm.set_sender(BOB);
        contract
            .transfer_from(ALICE, OWNER, U256::from(500))
            .unwrap();

        // Infinite allowance should not decrease
        assert_eq!(contract.allowance(ALICE, BOB), U256::MAX);
        assert_eq!(contract.balance_of(ALICE), U256::from(500));
    }

    #[test]
    fn test_transfer_from_when_paused_reverts() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(1000));

        vm.set_sender(ALICE);
        contract.approve(BOB, U256::from(500)).unwrap();

        vm.set_sender(OWNER);
        contract.pause().unwrap();

        vm.set_sender(BOB);
        let result = contract.transfer_from(ALICE, OWNER, U256::from(100));
        assert!(matches!(result, Err(TokenError::ContractPaused(_))));
    }

    // ─── burn ────────────────────────────────────────────────────────

    #[test]
    fn test_burn_success() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(1000));
        vm.set_sender(ALICE);

        contract.burn(U256::from(400)).unwrap();

        assert_eq!(contract.balance_of(ALICE), U256::from(600));
        assert_eq!(contract.total_supply(), U256::from(600));
    }

    #[test]
    fn test_burn_insufficient_balance_reverts() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(100));
        vm.set_sender(ALICE);

        let result = contract.burn(U256::from(200));
        assert!(matches!(result, Err(TokenError::InsufficientBalance(_))));
    }

    #[test]
    fn test_burn_zero_amount_reverts() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(100));
        vm.set_sender(ALICE);

        let result = contract.burn(U256::ZERO);
        assert!(matches!(result, Err(TokenError::ZeroAmount(_))));
    }

    #[test]
    fn test_burn_when_paused_reverts() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(100));
        contract.pause().unwrap();

        vm.set_sender(ALICE);
        let result = contract.burn(U256::from(50));
        assert!(matches!(result, Err(TokenError::ContractPaused(_))));
    }

    // ─── pause / unpause ─────────────────────────────────────────────

    #[test]
    fn test_pause_success() {
        let (_, mut contract) = setup();

        contract.pause().unwrap();
        assert!(contract.is_paused());
    }

    #[test]
    fn test_unpause_success() {
        let (_, mut contract) = setup();

        contract.pause().unwrap();
        contract.unpause().unwrap();
        assert!(!contract.is_paused());
    }

    #[test]
    fn test_pause_by_admin() {
        let (vm, mut contract) = setup();
        contract.grant_admin(ADMIN).unwrap();

        vm.set_sender(ADMIN);
        contract.pause().unwrap();
        assert!(contract.is_paused());
    }

    #[test]
    fn test_pause_unauthorized_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.pause();
        assert!(matches!(result, Err(TokenError::Unauthorized(_))));
    }

    #[test]
    fn test_pause_already_paused_reverts() {
        let (_, mut contract) = setup();
        contract.pause().unwrap();

        let result = contract.pause();
        assert!(matches!(result, Err(TokenError::ContractPaused(_))));
    }

    #[test]
    fn test_unpause_not_paused_reverts() {
        let (_, mut contract) = setup();

        let result = contract.unpause();
        assert!(matches!(result, Err(TokenError::ContractNotPaused(_))));
    }

    #[test]
    fn test_operations_resume_after_unpause() {
        let (vm, mut contract) = setup_with_tokens(ALICE, U256::from(1000));

        contract.pause().unwrap();
        contract.unpause().unwrap();

        vm.set_sender(ALICE);
        contract.transfer(BOB, U256::from(100)).unwrap();
        assert_eq!(contract.balance_of(BOB), U256::from(100));
    }

    // ─── Access Control ──────────────────────────────────────────────

    #[test]
    fn test_grant_admin_success() {
        let (_, mut contract) = setup();

        assert!(!contract.is_admin(ADMIN));
        contract.grant_admin(ADMIN).unwrap();
        assert!(contract.is_admin(ADMIN));
    }

    #[test]
    fn test_grant_admin_not_owner_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.grant_admin(ADMIN);
        assert!(matches!(result, Err(TokenError::Unauthorized(_))));
    }

    #[test]
    fn test_grant_admin_zero_address_reverts() {
        let (_, mut contract) = setup();

        let result = contract.grant_admin(Address::ZERO);
        assert!(matches!(result, Err(TokenError::ZeroAddress(_))));
    }

    #[test]
    fn test_revoke_admin_success() {
        let (_, mut contract) = setup();

        contract.grant_admin(ADMIN).unwrap();
        contract.revoke_admin(ADMIN).unwrap();
        assert!(!contract.is_admin(ADMIN));
    }

    #[test]
    fn test_grant_authorized_success() {
        let (_, mut contract) = setup();

        assert!(!contract.is_authorized(MINTER));
        contract.grant_authorized(MINTER).unwrap();
        assert!(contract.is_authorized(MINTER));
    }

    #[test]
    fn test_revoke_authorized_cannot_mint() {
        let (vm, mut contract) = setup();

        contract.grant_authorized(MINTER).unwrap();
        vm.set_sender(MINTER);
        contract.mint(ALICE, U256::from(100)).unwrap();

        vm.set_sender(OWNER);
        contract.revoke_authorized(MINTER).unwrap();

        vm.set_sender(MINTER);
        let result = contract.mint(ALICE, U256::from(100));
        assert!(matches!(result, Err(TokenError::Unauthorized(_))));
    }

    // ─── View Defaults ───────────────────────────────────────────────

    #[test]
    fn test_balance_of_default_zero() {
        let (_, contract) = setup();
        assert_eq!(contract.balance_of(ALICE), U256::ZERO);
    }

    #[test]
    fn test_allowance_default_zero() {
        let (_, contract) = setup();
        assert_eq!(contract.allowance(ALICE, BOB), U256::ZERO);
    }

    #[test]
    fn test_non_admin_not_admin() {
        let (_, contract) = setup();
        assert!(!contract.is_admin(ALICE));
    }

    #[test]
    fn test_non_authorized_not_authorized() {
        let (_, contract) = setup();
        assert!(!contract.is_authorized(ALICE));
    }
}
