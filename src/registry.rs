//! DIURegistry — User identity, ORCID linking, researcher verification.
//!
//! Central registry for all DIU OS user accounts. Users self-register with a
//! metadata URI (IPFS/Arweave), can link their ORCID identifier, and admins
//! can verify researchers.

use alloc::string::String;
use stylus_sdk::{
    alloy_primitives::{keccak256, Address, U256},
    alloy_sol_types::sol,
    prelude::*,
};

// ═══════════════════════════════════════════════════════════════════════════
// EVENTS
// ═══════════════════════════════════════════════════════════════════════════

sol! {
    /// Emitted when the contract is initialized.
    event Initialized(address indexed owner);

    /// Emitted when a new user registers.
    event UserRegistered(address indexed user, string metadata_uri);

    /// Emitted when a user updates their profile metadata.
    event ProfileUpdated(address indexed user, string metadata_uri);

    /// Emitted when a user links their ORCID identifier.
    event OrcidLinked(address indexed user, string orcid_id);

    /// Emitted when an admin verifies a researcher.
    event ResearcherVerified(address indexed user, address indexed admin);

    /// Emitted when the owner grants admin role to an account.
    event AdminGranted(address indexed account, address indexed granted_by);

    /// Emitted when the owner revokes admin role from an account.
    event AdminRevoked(address indexed account, address indexed revoked_by);
}

// ═══════════════════════════════════════════════════════════════════════════
// ERRORS
// ═══════════════════════════════════════════════════════════════════════════

sol! {
    error Unauthorized();
    error AlreadyRegistered();
    error NotRegistered();
    error EmptyString();
    error AlreadyVerified();
    error OrcidAlreadyLinked();
    /// This ORCID is already linked by a different address.
    error OrcidAlreadyRegistered();
    error ZeroAddress();
    error AlreadyInitialized();
}

/// Contract-level error type for DIURegistry.
#[derive(SolidityError)]
pub enum RegistryError {
    Unauthorized(Unauthorized),
    AlreadyRegistered(AlreadyRegistered),
    NotRegistered(NotRegistered),
    EmptyString(EmptyString),
    AlreadyVerified(AlreadyVerified),
    OrcidAlreadyLinked(OrcidAlreadyLinked),
    OrcidAlreadyRegistered(OrcidAlreadyRegistered),
    ZeroAddress(ZeroAddress),
    AlreadyInitialized(AlreadyInitialized),
}

impl core::fmt::Debug for RegistryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unauthorized(_) => write!(f, "Unauthorized"),
            Self::AlreadyRegistered(_) => write!(f, "AlreadyRegistered"),
            Self::NotRegistered(_) => write!(f, "NotRegistered"),
            Self::EmptyString(_) => write!(f, "EmptyString"),
            Self::AlreadyVerified(_) => write!(f, "AlreadyVerified"),
            Self::OrcidAlreadyLinked(_) => write!(f, "OrcidAlreadyLinked"),
            Self::OrcidAlreadyRegistered(_) => write!(f, "OrcidAlreadyRegistered"),
            Self::ZeroAddress(_) => write!(f, "ZeroAddress"),
            Self::AlreadyInitialized(_) => write!(f, "AlreadyInitialized"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// STORAGE
// ═══════════════════════════════════════════════════════════════════════════

sol_storage! {
    #[entrypoint]
    pub struct DIURegistry {
        /// Contract owner (set at deployment via constructor).
        address owner;

        /// Whether the contract has been initialized.
        bool initialized;

        /// Addresses with admin privileges.
        mapping(address => bool) admins;

        /// Whether an address has registered.
        mapping(address => bool) registered;

        /// User metadata URI (IPFS/Arweave).
        mapping(address => string) metadata_uris;

        /// ORCID identifier linked to a user.
        mapping(address => string) orcid_ids;

        /// Global ORCID uniqueness index: keccak256(orcid_id) → whether taken.
        ///
        /// Prevents the same ORCID string from being linked to multiple addresses.
        mapping(bytes32 => bool) orcid_registry;

        /// Whether a user is a verified researcher.
        mapping(address => bool) verified;

        /// Block timestamp when the user registered.
        mapping(address => uint256) registered_at;

        /// Ordered list of all registered user addresses.
        address[] user_list;

        /// Total number of registered users.
        uint256 total_users;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// INTERNAL HELPERS
// ═══════════════════════════════════════════════════════════════════════════

impl DIURegistry {
    /// Returns an error if the caller is not the contract owner.
    fn require_owner(&self) -> Result<(), RegistryError> {
        if self.vm().msg_sender() != self.owner.get() {
            return Err(RegistryError::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }

    /// Returns an error if the caller is not an admin (owner counts as admin).
    fn require_admin(&self) -> Result<(), RegistryError> {
        let caller = self.vm().msg_sender();
        if caller != self.owner.get() && !self.admins.get(caller) {
            return Err(RegistryError::Unauthorized(Unauthorized {}));
        }
        Ok(())
    }

    /// Returns an error if the given user is not registered.
    fn require_registered(&self, user: Address) -> Result<(), RegistryError> {
        if !self.registered.get(user) {
            return Err(RegistryError::NotRegistered(NotRegistered {}));
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC INTERFACE
// ═══════════════════════════════════════════════════════════════════════════

#[public]
impl DIURegistry {
    // ─── Initialization ──────────────────────────────────────────────

    /// Initialize the contract. Can only be called once.
    ///
    /// Sets the caller as owner and grants initial admin role.
    pub fn initialize(&mut self) -> Result<(), RegistryError> {
        if self.initialized.get() {
            return Err(RegistryError::AlreadyInitialized(AlreadyInitialized {}));
        }

        let caller = self.vm().msg_sender();
        self.owner.set(caller);
        self.admins.setter(caller).set(true);
        self.initialized.set(true);

        self.vm().log(Initialized { owner: caller });
        Ok(())
    }

    // ─── Write Functions ─────────────────────────────────────────────

    /// Register a new user account with the given metadata URI.
    ///
    /// The caller becomes the registered user. Each address can only
    /// register once.
    pub fn register_user(&mut self, metadata_uri: String) -> Result<(), RegistryError> {
        if metadata_uri.is_empty() {
            return Err(RegistryError::EmptyString(EmptyString {}));
        }

        let caller = self.vm().msg_sender();

        if self.registered.get(caller) {
            return Err(RegistryError::AlreadyRegistered(AlreadyRegistered {}));
        }

        self.registered.setter(caller).set(true);
        self.metadata_uris.setter(caller).set_str(&metadata_uri);

        let ts = U256::from(self.vm().block_timestamp());
        self.registered_at.setter(caller).set(ts);

        self.user_list.push(caller);
        let count = self.total_users.get();
        self.total_users.set(count + U256::from(1));

        self.vm().log(UserRegistered {
            user: caller,
            metadata_uri,
        });

        Ok(())
    }

    /// Update the caller's profile metadata URI.
    pub fn update_profile(&mut self, metadata_uri: String) -> Result<(), RegistryError> {
        if metadata_uri.is_empty() {
            return Err(RegistryError::EmptyString(EmptyString {}));
        }

        let caller = self.vm().msg_sender();
        self.require_registered(caller)?;

        self.metadata_uris.setter(caller).set_str(&metadata_uri);

        self.vm().log(ProfileUpdated {
            user: caller,
            metadata_uri,
        });

        Ok(())
    }

    /// Link an ORCID identifier to the caller's account.
    ///
    /// Phase 1: stores the ORCID string without cryptographic signature
    /// verification. Signature verification deferred pending security
    /// advisor guidance.
    ///
    /// Enforces global uniqueness: each ORCID can be linked to at most one address.
    pub fn link_orcid(&mut self, orcid_id: String) -> Result<(), RegistryError> {
        if orcid_id.is_empty() {
            return Err(RegistryError::EmptyString(EmptyString {}));
        }

        let caller = self.vm().msg_sender();
        self.require_registered(caller)?;

        // Caller already has an ORCID linked.
        if !self.orcid_ids.get(caller).get_string().is_empty() {
            return Err(RegistryError::OrcidAlreadyLinked(OrcidAlreadyLinked {}));
        }

        // Global uniqueness: reject if this ORCID is already claimed by any address.
        let orcid_hash = keccak256(orcid_id.as_bytes());
        if self.orcid_registry.get(orcid_hash) {
            return Err(RegistryError::OrcidAlreadyRegistered(
                OrcidAlreadyRegistered {},
            ));
        }

        self.orcid_ids.setter(caller).set_str(&orcid_id);
        self.orcid_registry.setter(orcid_hash).set(true);

        self.vm().log(OrcidLinked {
            user: caller,
            orcid_id,
        });

        Ok(())
    }

    /// Mark a registered user as a verified researcher. Admin only.
    pub fn verify_researcher(&mut self, user: Address) -> Result<(), RegistryError> {
        self.require_admin()?;
        self.require_registered(user)?;

        if self.verified.get(user) {
            return Err(RegistryError::AlreadyVerified(AlreadyVerified {}));
        }

        self.verified.setter(user).set(true);

        let admin = self.vm().msg_sender();
        self.vm().log(ResearcherVerified { user, admin });

        Ok(())
    }

    /// Grant admin role to an account. Owner only.
    pub fn grant_admin(&mut self, account: Address) -> Result<(), RegistryError> {
        self.require_owner()?;

        if account == Address::ZERO {
            return Err(RegistryError::ZeroAddress(ZeroAddress {}));
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
    pub fn revoke_admin(&mut self, account: Address) -> Result<(), RegistryError> {
        self.require_owner()?;
        self.admins.setter(account).set(false);

        let revoked_by = self.vm().msg_sender();
        self.vm().log(AdminRevoked {
            account,
            revoked_by,
        });

        Ok(())
    }

    // ─── View Functions ──────────────────────────────────────────────

    /// Get user data: (metadata_uri, orcid_id, is_verified, registered_at).
    ///
    /// Returns default values if the user is not registered.
    pub fn get_user(&self, user: Address) -> (String, String, bool, U256) {
        let metadata_uri = self.metadata_uris.get(user).get_string();
        let orcid_id = self.orcid_ids.get(user).get_string();
        let is_verified = self.verified.get(user);
        let registered_at = self.registered_at.get(user);

        (metadata_uri, orcid_id, is_verified, registered_at)
    }

    /// Check whether an address is registered.
    pub fn is_registered(&self, user: Address) -> bool {
        self.registered.get(user)
    }

    /// Check whether a user is a verified researcher.
    pub fn is_verified(&self, user: Address) -> bool {
        self.verified.get(user)
    }

    /// Get the total number of registered users.
    pub fn total_users(&self) -> U256 {
        self.total_users.get()
    }

    /// Get the contract owner address.
    pub fn owner(&self) -> Address {
        self.owner.get()
    }

    /// Check whether an address has admin privileges.
    pub fn is_admin(&self, account: Address) -> bool {
        account == self.owner.get() || self.admins.get(account)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use stylus_sdk::testing::*;
    use alloy_primitives::address;

    const OWNER: Address = address!("1111111111111111111111111111111111111111");
    const ALICE: Address = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    const BOB: Address = address!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

    /// Helper: create a VM and initialized contract with OWNER as owner.
    fn setup() -> (TestVM, DIURegistry) {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIURegistry::from(&vm);
        contract.initialize().unwrap();
        (vm, contract)
    }

    // ─── Initialization ──────────────────────────────────────────────

    #[test]
    fn test_initialize_sets_owner() {
        let (_, contract) = setup();
        assert_eq!(contract.owner(), OWNER);
    }

    #[test]
    fn test_initialize_grants_admin() {
        let (_, contract) = setup();
        assert!(contract.is_admin(OWNER));
    }

    #[test]
    fn test_initialize_starts_empty() {
        let (_, contract) = setup();
        assert_eq!(contract.total_users(), U256::ZERO);
    }

    #[test]
    fn test_initialize_twice_reverts() {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIURegistry::from(&vm);
        contract.initialize().unwrap();

        let result = contract.initialize();
        assert!(matches!(result, Err(RegistryError::AlreadyInitialized(_))));
    }

    #[test]
    fn test_initialize_sets_initialized() {
        let vm = TestVMBuilder::new().sender(OWNER).build();
        let mut contract = DIURegistry::from(&vm);

        contract.initialize().unwrap();

        // Verify by trying to initialize again
        let result = contract.initialize();
        assert!(matches!(result, Err(RegistryError::AlreadyInitialized(_))));
    }

    // ─── register_user ───────────────────────────────────────────────

    #[test]
    fn test_register_user_success() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.register_user("ipfs://alice-meta".into());
        assert!(result.is_ok());

        assert!(contract.is_registered(ALICE));
        assert_eq!(contract.total_users(), U256::from(1));

        let (uri, orcid, verified, _ts) = contract.get_user(ALICE);
        assert_eq!(uri, "ipfs://alice-meta");
        assert_eq!(orcid, "");
        assert!(!verified);
    }

    #[test]
    fn test_register_user_empty_uri_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.register_user("".into());
        assert!(matches!(result, Err(RegistryError::EmptyString(_))));
    }

    #[test]
    fn test_register_user_duplicate_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);
        contract.register_user("uri1".into()).unwrap();

        let result = contract.register_user("uri2".into());
        assert!(matches!(result, Err(RegistryError::AlreadyRegistered(_))));
    }

    #[test]
    fn test_register_multiple_users() {
        let (vm, mut contract) = setup();

        vm.set_sender(ALICE);
        contract.register_user("uri-alice".into()).unwrap();

        vm.set_sender(BOB);
        contract.register_user("uri-bob".into()).unwrap();

        assert_eq!(contract.total_users(), U256::from(2));
        assert!(contract.is_registered(ALICE));
        assert!(contract.is_registered(BOB));
    }

    // ─── update_profile ──────────────────────────────────────────────

    #[test]
    fn test_update_profile_success() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);
        contract.register_user("old-uri".into()).unwrap();

        contract.update_profile("new-uri".into()).unwrap();

        let (uri, _, _, _) = contract.get_user(ALICE);
        assert_eq!(uri, "new-uri");
    }

    #[test]
    fn test_update_profile_not_registered_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.update_profile("uri".into());
        assert!(matches!(result, Err(RegistryError::NotRegistered(_))));
    }

    #[test]
    fn test_update_profile_empty_uri_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);
        contract.register_user("uri".into()).unwrap();

        let result = contract.update_profile("".into());
        assert!(matches!(result, Err(RegistryError::EmptyString(_))));
    }

    // ─── link_orcid ──────────────────────────────────────────────────

    #[test]
    fn test_link_orcid_success() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);
        contract.register_user("uri".into()).unwrap();

        contract.link_orcid("0000-0001-2345-6789".into()).unwrap();

        let (_, orcid, _, _) = contract.get_user(ALICE);
        assert_eq!(orcid, "0000-0001-2345-6789");
    }

    #[test]
    fn test_link_orcid_not_registered_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.link_orcid("0000-0001-2345-6789".into());
        assert!(matches!(result, Err(RegistryError::NotRegistered(_))));
    }

    #[test]
    fn test_link_orcid_empty_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);
        contract.register_user("uri".into()).unwrap();

        let result = contract.link_orcid("".into());
        assert!(matches!(result, Err(RegistryError::EmptyString(_))));
    }

    #[test]
    fn test_link_orcid_already_linked_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);
        contract.register_user("uri".into()).unwrap();
        contract.link_orcid("0000-0001-2345-6789".into()).unwrap();

        let result = contract.link_orcid("0000-0001-9999-9999".into());
        assert!(matches!(result, Err(RegistryError::OrcidAlreadyLinked(_))));
    }

    #[test]
    fn test_link_orcid_same_orcid_two_addresses_reverts() {
        let (vm, mut contract) = setup();

        // ALICE links the ORCID first.
        vm.set_sender(ALICE);
        contract.register_user("uri-alice".into()).unwrap();
        contract.link_orcid("0000-0001-2345-6789".into()).unwrap();

        // BOB tries to link the same ORCID — must fail.
        vm.set_sender(BOB);
        contract.register_user("uri-bob".into()).unwrap();
        let result = contract.link_orcid("0000-0001-2345-6789".into());
        assert!(matches!(
            result,
            Err(RegistryError::OrcidAlreadyRegistered(_))
        ));
    }

    // ─── verify_researcher ───────────────────────────────────────────

    #[test]
    fn test_verify_researcher_success() {
        let (vm, mut contract) = setup();

        vm.set_sender(ALICE);
        contract.register_user("uri".into()).unwrap();

        vm.set_sender(OWNER);
        contract.verify_researcher(ALICE).unwrap();

        assert!(contract.is_verified(ALICE));
    }

    #[test]
    fn test_verify_not_admin_reverts() {
        let (vm, mut contract) = setup();

        vm.set_sender(ALICE);
        contract.register_user("uri".into()).unwrap();

        vm.set_sender(BOB);
        let result = contract.verify_researcher(ALICE);
        assert!(matches!(result, Err(RegistryError::Unauthorized(_))));
    }

    #[test]
    fn test_verify_not_registered_reverts() {
        let (_, mut contract) = setup();

        let result = contract.verify_researcher(ALICE);
        assert!(matches!(result, Err(RegistryError::NotRegistered(_))));
    }

    #[test]
    fn test_verify_already_verified_reverts() {
        let (vm, mut contract) = setup();

        vm.set_sender(ALICE);
        contract.register_user("uri".into()).unwrap();

        vm.set_sender(OWNER);
        contract.verify_researcher(ALICE).unwrap();

        let result = contract.verify_researcher(ALICE);
        assert!(matches!(result, Err(RegistryError::AlreadyVerified(_))));
    }

    // ─── grant_admin / revoke_admin ──────────────────────────────────

    #[test]
    fn test_grant_admin_success() {
        let (_, mut contract) = setup();

        assert!(!contract.is_admin(ALICE));
        contract.grant_admin(ALICE).unwrap();
        assert!(contract.is_admin(ALICE));
    }

    #[test]
    fn test_grant_admin_not_owner_reverts() {
        let (vm, mut contract) = setup();
        vm.set_sender(ALICE);

        let result = contract.grant_admin(BOB);
        assert!(matches!(result, Err(RegistryError::Unauthorized(_))));
    }

    #[test]
    fn test_grant_admin_zero_address_reverts() {
        let (_, mut contract) = setup();

        let result = contract.grant_admin(Address::ZERO);
        assert!(matches!(result, Err(RegistryError::ZeroAddress(_))));
    }

    #[test]
    fn test_revoke_admin_success() {
        let (_, mut contract) = setup();

        contract.grant_admin(ALICE).unwrap();
        assert!(contract.is_admin(ALICE));

        contract.revoke_admin(ALICE).unwrap();
        assert!(!contract.is_admin(ALICE));
    }

    #[test]
    fn test_admin_can_verify() {
        let (vm, mut contract) = setup();

        // Grant admin to ALICE
        contract.grant_admin(ALICE).unwrap();

        // BOB registers
        vm.set_sender(BOB);
        contract.register_user("uri".into()).unwrap();

        // ALICE (admin) verifies BOB
        vm.set_sender(ALICE);
        contract.verify_researcher(BOB).unwrap();
        assert!(contract.is_verified(BOB));
    }

    // ─── View functions ──────────────────────────────────────────────

    #[test]
    fn test_get_user_not_registered() {
        let (_, contract) = setup();

        let (uri, orcid, verified, ts) = contract.get_user(ALICE);
        assert_eq!(uri, "");
        assert_eq!(orcid, "");
        assert!(!verified);
        assert_eq!(ts, U256::ZERO);
    }

    #[test]
    fn test_is_registered_false() {
        let (_, contract) = setup();
        assert!(!contract.is_registered(ALICE));
    }

    #[test]
    fn test_is_verified_false() {
        let (_, contract) = setup();
        assert!(!contract.is_verified(ALICE));
    }
}
