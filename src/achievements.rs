//! DIUAchievements — Soulbound ERC-721 NFT badges and certificates.
//!
//! Non-transferable achievement tokens for the DIU OS platform.
//! Authorized backend addresses mint achievements; each user can earn
//! each achievement type exactly once. Implements ERC-721 view interface
//! with transfers disabled (soulbound).

use alloc::string::String;
use alloc::vec::Vec;
use stylus_sdk::{
    alloy_primitives::{Address, U256},
    alloy_sol_types::sol,
    prelude::*,
};

// ═══════════════════════════════════════════════════════════════════════════
// EVENTS
// ═══════════════════════════════════════════════════════════════════════════

sol! {
    /// ERC-721 standard Transfer event (from=0x0 on mint).
    event Transfer(address indexed from, address indexed to, uint256 indexed token_id);

    /// Emitted when an achievement NFT is minted.
    event AchievementMinted(
        address indexed user,
        uint256 indexed achievement_id,
        uint256 token_id,
        string metadata_uri
    );

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
    error TokenNotFound();
    error AchievementAlreadyEarned();
    error EmptyString();
    error Soulbound();
}

/// Contract-level error type for DIUAchievements.
#[derive(SolidityError)]
pub enum AchievementsError {
    Unauthorized(Unauthorized),
    ZeroAddress(ZeroAddress),
    TokenNotFound(TokenNotFound),
    AchievementAlreadyEarned(AchievementAlreadyEarned),
    EmptyString(EmptyString),
    Soulbound(Soulbound),
}

impl core::fmt::Debug for AchievementsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unauthorized(_) => write!(f, "Unauthorized"),
            Self::ZeroAddress(_) => write!(f, "ZeroAddress"),
            Self::TokenNotFound(_) => write!(f, "TokenNotFound"),
            Self::AchievementAlreadyEarned(_) => write!(f, "AchievementAlreadyEarned"),
            Self::EmptyString(_) => write!(f, "EmptyString"),
            Self::Soulbound(_) => write!(f, "Soulbound"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STORAGE
// ═══════════════════════════════════════════════════════════════════════════

sol_storage! {
    #[entrypoint]
    pub struct DIUAchievements {
        /// Contract owner (set at deployment).
        address owner;

        /// Authorized backend addresses that can mint achievements.
        mapping(address => bool) authorized;

        // ─── ERC-721 core (soulbound — read-only after mint) ─────────

        /// Token ID → owner address.
        mapping(uint256 => address) token_owners;

        /// Owner address → token balance.
        mapping(address => uint256) balances;

        /// Token ID → metadata URI.
        mapping(uint256 => string) token_uris;

        // ─── Achievement tracking ────────────────────────────────────

        /// (user, achievement_id) → whether earned.
        mapping(address => mapping(uint256 => bool)) achieved;

        /// (user, achievement_id) → token ID.
        mapping(address => mapping(uint256 => uint256)) achievement_tokens;

        /// Token ID → achievement ID.
        mapping(uint256 => uint256) token_achievement;

        // ─── Per-user token enumeration (append-only) ────────────────

        /// User → number of tokens held.
        mapping(address => uint256) user_token_count;

        /// (user, index) → token ID.
        mapping(address => mapping(uint256 => uint256)) user_token_at;

        // ─── Counters ────────────────────────────────────────────────

        /// Next token ID to mint (starts at 1).
        uint256 next_token_id;

        /// Total number of tokens minted.
        uint256 total_supply;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INTERNAL HELPERS
// ═══════════════════════════════════════════════════════════════════════════

impl DIUAchievements {
    /// Returns an error if the caller is not the contract owner.
    fn require_owner(&self) -> Result<(), AchievementsError> {
        if self.vm().msg_sender() != self.owner.get() {
            return Err(AchievementsError::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }

    /// Returns an error if the caller is not authorized (owner counts as authorized).
    fn require_authorized(&self) -> Result<(), AchievementsError> {
        let caller = self.vm().msg_sender();
        if caller != self.owner.get() && !self.authorized.get(caller) {
            return Err(AchievementsError::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }

    /// Returns an error if the token does not exist.
    fn require_token_exists(&self, token_id: U256) -> Result<(), AchievementsError> {
        if self.token_owners.get(token_id) == Address::ZERO {
            return Err(AchievementsError::TokenNotFound(TokenNotFound {}));
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CONSTRUCTOR
// ═══════════════════════════════════════════════════════════════════════════

impl DIUAchievements {
    /// Initializes the contract with the deployer as owner and authorized.
    /// Token IDs start at 1.
    pub fn constructor(&mut self) {
        let deployer = self.vm().tx_origin();
        self.owner.set(deployer);
        self.authorized.setter(deployer).set(true);
        self.next_token_id.set(U256::from(1));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC INTERFACE
// ═══════════════════════════════════════════════════════════════════════════

#[public]
impl DIUAchievements {
    // ─── Mint (Authorized Only) ──────────────────────────────────────

    /// Mint an achievement NFT to a user. Authorized callers only.
    ///
    /// Each (user, achievement_id) pair can only be minted once.
    /// Returns the newly minted token ID.
    pub fn mint(
        &mut self,
        user: Address,
        achievement_id: U256,
        metadata_uri: String,
    ) -> Result<U256, AchievementsError> {
        self.require_authorized()?;

        if user == Address::ZERO {
            return Err(AchievementsError::ZeroAddress(ZeroAddress {}));
        }
        if metadata_uri.is_empty() {
            return Err(AchievementsError::EmptyString(EmptyString {}));
        }

        // Check user hasn't already earned this achievement
        if self.achieved.getter(user).get(achievement_id) {
            return Err(AchievementsError::AchievementAlreadyEarned(
                AchievementAlreadyEarned {},
            ));
        }

        // Allocate token ID
        let token_id = self.next_token_id.get();
        self.next_token_id.set(token_id + U256::from(1));

        // Set token owner and metadata
        self.token_owners.setter(token_id).set(user);
        self.token_uris.setter(token_id).set_str(&metadata_uri);

        // Update balance
        let balance = self.balances.get(user);
        self.balances.setter(user).set(balance + U256::from(1));

        // Track achievement
        self.achieved.setter(user).setter(achievement_id).set(true);
        self.achievement_tokens
            .setter(user)
            .setter(achievement_id)
            .set(token_id);
        self.token_achievement.setter(token_id).set(achievement_id);

        // Append to user's token list
        let count = self.user_token_count.get(user);
        self.user_token_at.setter(user).setter(count).set(token_id);
        self.user_token_count.setter(user).set(count + U256::from(1));

        // Update total supply
        let supply = self.total_supply.get();
        self.total_supply.set(supply + U256::from(1));

        // Emit ERC-721 Transfer (from zero = mint)
        self.vm().log(Transfer {
            from: Address::ZERO,
            to: user,
            token_id,
        });

        self.vm().log(AchievementMinted {
            user,
            achievement_id,
            token_id,
            metadata_uri,
        });

        Ok(token_id)
    }

    // ─── ERC-721 Transfer Functions (Soulbound — always revert) ──────

    /// Disabled — tokens are soulbound (non-transferable).
    pub fn transfer_from(
        &mut self,
        _from: Address,
        _to: Address,
        _token_id: U256,
    ) -> Result<(), AchievementsError> {
        Err(AchievementsError::Soulbound(Soulbound {}))
    }

    /// Disabled — tokens are soulbound (non-transferable).
    pub fn approve(
        &mut self,
        _to: Address,
        _token_id: U256,
    ) -> Result<(), AchievementsError> {
        Err(AchievementsError::Soulbound(Soulbound {}))
    }

    /// Disabled — tokens are soulbound (non-transferable).
    pub fn set_approval_for_all(
        &mut self,
        _operator: Address,
        _approved: bool,
    ) -> Result<(), AchievementsError> {
        Err(AchievementsError::Soulbound(Soulbound {}))
    }

    // ─── Access Control (Owner Only) ─────────────────────────────────

    /// Grant authorized role to an account. Owner only.
    pub fn grant_authorized(&mut self, account: Address) -> Result<(), AchievementsError> {
        self.require_owner()?;

        if account == Address::ZERO {
            return Err(AchievementsError::ZeroAddress(ZeroAddress {}));
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
    pub fn revoke_authorized(&mut self, account: Address) -> Result<(), AchievementsError> {
        self.require_owner()?;

        self.authorized.setter(account).set(false);

        let revoked_by = self.vm().msg_sender();
        self.vm().log(AuthorizedRevoked {
            account,
            revoked_by,
        });

        Ok(())
    }

    // ─── ERC-721 View Functions ──────────────────────────────────────

    /// Get the number of tokens owned by an address.
    pub fn balance_of(&self, owner: Address) -> U256 {
        self.balances.get(owner)
    }

    /// Get the owner of a token. Returns zero address if token does not exist.
    pub fn owner_of(&self, token_id: U256) -> Address {
        self.token_owners.get(token_id)
    }

    /// Get the metadata URI for a token.
    #[selector(name = "tokenURI")]
    pub fn token_uri(&self, token_id: U256) -> Result<String, AchievementsError> {
        self.require_token_exists(token_id)?;
        Ok(self.token_uris.get(token_id).get_string())
    }

    /// Returns zero address — approvals disabled for soulbound tokens.
    pub fn get_approved(&self, _token_id: U256) -> Address {
        Address::ZERO
    }

    /// Returns false — operator approvals disabled for soulbound tokens.
    pub fn is_approved_for_all(&self, _owner: Address, _operator: Address) -> bool {
        false
    }

    /// Collection name.
    pub fn name(&self) -> String {
        String::from("DIU Achievements")
    }

    /// Collection symbol.
    pub fn symbol(&self) -> String {
        String::from("DIUA")
    }

    // ─── Achievement View Functions ──────────────────────────────────

    /// Check whether a user has earned a specific achievement.
    pub fn has_achievement(&self, user: Address, achievement_id: U256) -> bool {
        self.achieved.getter(user).get(achievement_id)
    }

    /// Get the token ID for a user's achievement. Returns 0 if not earned.
    pub fn get_achievement_token(&self, user: Address, achievement_id: U256) -> U256 {
        self.achievement_tokens.getter(user).get(achievement_id)
    }

    /// Get the achievement ID for a token.
    pub fn get_token_achievement(&self, token_id: U256) -> U256 {
        self.token_achievement.get(token_id)
    }

    /// Get all token IDs owned by a user.
    pub fn get_achievements(&self, user: Address) -> Vec<U256> {
        let count = self.user_token_count.get(user);
        let n = count.saturating_to::<usize>();

        let mut tokens = Vec::with_capacity(n);
        for i in 0..n {
            let token_id = self.user_token_at.getter(user).get(U256::from(i));
            tokens.push(token_id);
        }
        tokens
    }

    /// Get the total number of tokens minted.
    pub fn total_supply(&self) -> U256 {
        self.total_supply.get()
    }

    /// Get the contract owner address.
    pub fn owner(&self) -> Address {
        self.owner.get()
    }

    /// Check whether an address is authorized.
    pub fn is_authorized(&self, account: Address) -> bool {
        account == self.owner.get() || self.authorized.get(account)
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

    /// Achievement IDs used in tests.
    const ACH_FIRST_EXPERIMENT: U256 = U256::from_limbs([1, 0, 0, 0]);
    const ACH_PERFECT_QUIZ: U256 = U256::from_limbs([2, 0, 0, 0]);
    const ACH_FIVE_DAY_STREAK: U256 = U256::from_limbs([3, 0, 0, 0]);

    /// Helper: create a VM and initialized contract with OWNER as owner.
    fn setup() -> (TestVM, DIUAchievements) {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIUAchievements::from(&vm);
        contract.constructor();
        (vm, contract)
    }

    /// Helper: setup with an authorized BACKEND address.
    fn setup_with_backend() -> (TestVM, DIUAchievements) {
        let (vm, mut contract) = setup();
        contract.grant_authorized(BACKEND).unwrap();
        vm.set_sender(BACKEND);
        (vm, contract)
    }

    // ─── Constructor ─────────────────────────────────────────────────

    #[test]
    fn test_constructor_sets_owner() {
        let (_, contract) = setup();
        assert_eq!(contract.owner(), OWNER);
    }

    #[test]
    fn test_constructor_owner_is_authorized() {
        let (_, contract) = setup();
        assert!(contract.is_authorized(OWNER));
    }

    #[test]
    fn test_constructor_starts_empty() {
        let (_, contract) = setup();
        assert_eq!(contract.total_supply(), U256::ZERO);
    }

    #[test]
    fn test_constructor_name_and_symbol() {
        let (_, contract) = setup();
        assert_eq!(contract.name(), "DIU Achievements");
        assert_eq!(contract.symbol(), "DIUA");
    }

    // ─── mint ────────────────────────────────────────────────────────

    #[test]
    fn test_mint_success() {
        let (_, mut contract) = setup_with_backend();

        let token_id = contract
            .mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://first-exp".into())
            .unwrap();

        assert_eq!(token_id, U256::from(1));
        assert_eq!(contract.owner_of(token_id), ALICE);
        assert_eq!(contract.balance_of(ALICE), U256::from(1));
        assert_eq!(contract.total_supply(), U256::from(1));
    }

    #[test]
    fn test_mint_sets_metadata() {
        let (_, mut contract) = setup_with_backend();

        let token_id = contract
            .mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://first-exp".into())
            .unwrap();

        let uri = contract.token_uri(token_id).unwrap();
        assert_eq!(uri, "ipfs://first-exp");
    }

    #[test]
    fn test_mint_tracks_achievement() {
        let (_, mut contract) = setup_with_backend();

        let token_id = contract
            .mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://meta".into())
            .unwrap();

        assert!(contract.has_achievement(ALICE, ACH_FIRST_EXPERIMENT));
        assert!(!contract.has_achievement(ALICE, ACH_PERFECT_QUIZ));
        assert_eq!(
            contract.get_achievement_token(ALICE, ACH_FIRST_EXPERIMENT),
            token_id
        );
        assert_eq!(contract.get_token_achievement(token_id), ACH_FIRST_EXPERIMENT);
    }

    #[test]
    fn test_mint_multiple_achievements_same_user() {
        let (_, mut contract) = setup_with_backend();

        let t1 = contract
            .mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://exp".into())
            .unwrap();
        let t2 = contract
            .mint(ALICE, ACH_PERFECT_QUIZ, "ipfs://quiz".into())
            .unwrap();

        assert_eq!(t1, U256::from(1));
        assert_eq!(t2, U256::from(2));
        assert_eq!(contract.balance_of(ALICE), U256::from(2));

        assert!(contract.has_achievement(ALICE, ACH_FIRST_EXPERIMENT));
        assert!(contract.has_achievement(ALICE, ACH_PERFECT_QUIZ));
    }

    #[test]
    fn test_mint_same_achievement_different_users() {
        let (_, mut contract) = setup_with_backend();

        let t1 = contract
            .mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://alice".into())
            .unwrap();
        let t2 = contract
            .mint(BOB, ACH_FIRST_EXPERIMENT, "ipfs://bob".into())
            .unwrap();

        assert_eq!(t1, U256::from(1));
        assert_eq!(t2, U256::from(2));
        assert_eq!(contract.owner_of(t1), ALICE);
        assert_eq!(contract.owner_of(t2), BOB);
    }

    #[test]
    fn test_mint_increments_token_ids() {
        let (_, mut contract) = setup_with_backend();

        let t1 = contract
            .mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://1".into())
            .unwrap();
        let t2 = contract
            .mint(BOB, ACH_PERFECT_QUIZ, "ipfs://2".into())
            .unwrap();
        let t3 = contract
            .mint(ALICE, ACH_FIVE_DAY_STREAK, "ipfs://3".into())
            .unwrap();

        assert_eq!(t1, U256::from(1));
        assert_eq!(t2, U256::from(2));
        assert_eq!(t3, U256::from(3));
        assert_eq!(contract.total_supply(), U256::from(3));
    }

    #[test]
    fn test_mint_duplicate_achievement_reverts() {
        let (_, mut contract) = setup_with_backend();

        contract
            .mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://1".into())
            .unwrap();

        let result = contract.mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://2".into());
        assert!(matches!(
            result,
            Err(AchievementsError::AchievementAlreadyEarned(_))
        ));
    }

    #[test]
    fn test_mint_unauthorized_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.mint(BOB, ACH_FIRST_EXPERIMENT, "ipfs://x".into());
        assert!(matches!(
            result,
            Err(AchievementsError::Unauthorized(_))
        ));
    }

    #[test]
    fn test_mint_zero_address_reverts() {
        let (_, mut contract) = setup_with_backend();

        let result = contract.mint(Address::ZERO, ACH_FIRST_EXPERIMENT, "ipfs://x".into());
        assert!(matches!(
            result,
            Err(AchievementsError::ZeroAddress(_))
        ));
    }

    #[test]
    fn test_mint_empty_uri_reverts() {
        let (_, mut contract) = setup_with_backend();

        let result = contract.mint(ALICE, ACH_FIRST_EXPERIMENT, "".into());
        assert!(matches!(
            result,
            Err(AchievementsError::EmptyString(_))
        ));
    }

    // ─── get_achievements ────────────────────────────────────────────

    #[test]
    fn test_get_achievements_empty() {
        let (_, contract) = setup();
        let tokens = contract.get_achievements(ALICE);
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_get_achievements_returns_all_tokens() {
        let (_, mut contract) = setup_with_backend();

        let t1 = contract
            .mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://1".into())
            .unwrap();
        let t2 = contract
            .mint(ALICE, ACH_PERFECT_QUIZ, "ipfs://2".into())
            .unwrap();
        let t3 = contract
            .mint(ALICE, ACH_FIVE_DAY_STREAK, "ipfs://3".into())
            .unwrap();

        let tokens = contract.get_achievements(ALICE);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], t1);
        assert_eq!(tokens[1], t2);
        assert_eq!(tokens[2], t3);
    }

    #[test]
    fn test_get_achievements_per_user() {
        let (_, mut contract) = setup_with_backend();

        contract
            .mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://a".into())
            .unwrap();
        contract
            .mint(BOB, ACH_FIRST_EXPERIMENT, "ipfs://b".into())
            .unwrap();
        contract
            .mint(ALICE, ACH_PERFECT_QUIZ, "ipfs://c".into())
            .unwrap();

        assert_eq!(contract.get_achievements(ALICE).len(), 2);
        assert_eq!(contract.get_achievements(BOB).len(), 1);
    }

    // ─── Soulbound (transfers disabled) ──────────────────────────────

    #[test]
    fn test_transfer_from_reverts() {
        let (_, mut contract) = setup_with_backend();

        contract
            .mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://x".into())
            .unwrap();

        let result = contract.transfer_from(ALICE, BOB, U256::from(1));
        assert!(matches!(result, Err(AchievementsError::Soulbound(_))));
    }

    #[test]
    fn test_approve_reverts() {
        let (_, mut contract) = setup();

        let result = contract.approve(ALICE, U256::from(1));
        assert!(matches!(result, Err(AchievementsError::Soulbound(_))));
    }

    #[test]
    fn test_set_approval_for_all_reverts() {
        let (_, mut contract) = setup();

        let result = contract.set_approval_for_all(ALICE, true);
        assert!(matches!(result, Err(AchievementsError::Soulbound(_))));
    }

    #[test]
    fn test_get_approved_returns_zero() {
        let (_, contract) = setup();
        assert_eq!(contract.get_approved(U256::from(1)), Address::ZERO);
    }

    #[test]
    fn test_is_approved_for_all_returns_false() {
        let (_, contract) = setup();
        assert!(!contract.is_approved_for_all(ALICE, BOB));
    }

    // ─── ERC-721 View Functions ──────────────────────────────────────

    #[test]
    fn test_token_uri_not_found_reverts() {
        let (_, contract) = setup();

        let result = contract.token_uri(U256::from(999));
        assert!(matches!(
            result,
            Err(AchievementsError::TokenNotFound(_))
        ));
    }

    #[test]
    fn test_balance_of_default_zero() {
        let (_, contract) = setup();
        assert_eq!(contract.balance_of(ALICE), U256::ZERO);
    }

    #[test]
    fn test_owner_of_nonexistent_returns_zero() {
        let (_, contract) = setup();
        assert_eq!(contract.owner_of(U256::from(999)), Address::ZERO);
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
        assert!(matches!(
            result,
            Err(AchievementsError::Unauthorized(_))
        ));
    }

    #[test]
    fn test_grant_authorized_zero_address_reverts() {
        let (_, mut contract) = setup();

        let result = contract.grant_authorized(Address::ZERO);
        assert!(matches!(
            result,
            Err(AchievementsError::ZeroAddress(_))
        ));
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
    fn test_revoked_cannot_mint() {
        let (vm, mut contract) = setup();

        contract.grant_authorized(BACKEND).unwrap();
        vm.set_sender(BACKEND);
        contract
            .mint(ALICE, ACH_FIRST_EXPERIMENT, "ipfs://x".into())
            .unwrap();

        vm.set_sender(OWNER);
        contract.revoke_authorized(BACKEND).unwrap();

        vm.set_sender(BACKEND);
        let result = contract.mint(BOB, ACH_FIRST_EXPERIMENT, "ipfs://y".into());
        assert!(matches!(
            result,
            Err(AchievementsError::Unauthorized(_))
        ));
    }

    // ─── Default View Values ─────────────────────────────────────────

    #[test]
    fn test_has_achievement_default_false() {
        let (_, contract) = setup();
        assert!(!contract.has_achievement(ALICE, ACH_FIRST_EXPERIMENT));
    }

    #[test]
    fn test_non_authorized_not_authorized() {
        let (_, contract) = setup();
        assert!(!contract.is_authorized(ALICE));
    }
}
