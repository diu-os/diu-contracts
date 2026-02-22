# DIU OS Smart Contracts

Smart contracts for [DIU OS](https://diu-os.org) — a decentralized Scientific Operating System for interactive quantum physics education, AI-assisted research, and Web3 integration.

Written in **Rust** using [Arbitrum Stylus SDK](https://docs.arbitrum.io/stylus), compiled to WASM, deployed on Arbitrum.

> Architecture and review by [Barust](https://github.com/barust)

## Contracts

Four Phase 1 contracts, each deployed as an independent WASM module via Cargo feature flags.

| Contract | File | Purpose | Tests | WASM |
|----------|------|---------|-------|------|
| **DIURegistry** | `src/registry.rs` | User identity, ORCID linking, researcher verification | 28 | 21.3 KB |
| **DIUReputation** | `src/reputation.rs` | XP tracking, levels, daily login streaks, leaderboard | 36 | 20.0 KB |
| **DIUAchievements** | `src/achievements.rs` | Soulbound ERC-721 NFT badges and certificates | 34 | 23.2 KB |
| **DIUToken** | `src/token.rs` | ERC-20 platform token with restricted mint and pause | 49 | 17.7 KB |

**Total: 147 tests, 0 clippy warnings.**

## Architecture

```
┌─────────────┐    ┌──────────────┐    ┌────────────────┐    ┌──────────┐
│ DIURegistry │    │DIUReputation │    │DIUAchievements │    │ DIUToken │
│─────────────│    │──────────────│    │────────────────│    │──────────│
│ User ID     │    │ XP & Levels  │    │ NFT Badges     │    │ ERC-20   │
│ ORCID link  │    │ Streaks      │───▶│ Certificates   │    │ Rewards  │
│ Verification│    │ Leaderboard  │    │ Soulbound      │    │ Pause    │
└─────────────┘    └──────┬───────┘    └────────────────┘    └────▲─────┘
                          │                                       │
                          └───────────────────────────────────────┘
                                  DIUReputation → DIUToken.mint
```

## Prerequisites

```bash
# Rust + WASM target
rustup target add wasm32-unknown-unknown

# Stylus CLI
cargo install cargo-stylus
```

## Build & Test

```bash
# Run all 147 tests
cargo test

# Lint (strict, zero warnings)
cargo clippy -- -D warnings

# Check WASM compilation for a specific contract
cargo stylus check --endpoint https://sepolia-rollup.arbitrum.io/rpc                     # DIURegistry (default)
cargo stylus check --features reputation --endpoint https://sepolia-rollup.arbitrum.io/rpc    # DIUReputation
cargo stylus check --features achievements --endpoint https://sepolia-rollup.arbitrum.io/rpc  # DIUAchievements
cargo stylus check --features token --endpoint https://sepolia-rollup.arbitrum.io/rpc         # DIUToken
```

## Feature-Gated Builds

Each contract has its own `#[entrypoint]` — only one is active per WASM build. Cargo features control which contract compiles:

```toml
[features]
reputation = []
achievements = []
token = []
# default (no feature) = DIURegistry
```

All contracts compile together in test mode (`cargo test` runs all 139 tests).

## Contract Details

### DIURegistry

User identity and profile management. Base contract with no dependencies.

| Function | Access | Description |
|----------|--------|-------------|
| `register_user(metadata_uri)` | public | Self-register with metadata URI |
| `update_profile(metadata_uri)` | registered user | Update own metadata |
| `link_orcid(orcid_id)` | registered user | Link ORCID researcher ID |
| `verify_researcher(user)` | admin | Mark user as verified |
| `grant_admin(account)` | owner | Grant admin role |
| `get_user(address)` | view | Get user profile data |

### DIUReputation

XP tracking, level calculation, daily login streaks, and leaderboard.

| Function | Access | Description |
|----------|--------|-------------|
| `add_xp(user, amount)` | authorized | Award XP to a user |
| `record_daily_login(user)` | authorized | Record login, update streak, +10 XP |
| `get_level(user)` | view | Level 1–5 based on XP |
| `get_streak(user)` | view | Current daily streak |
| `get_leaderboard(limit)` | view | Top N users by XP |

**Level thresholds**: 1 (0 XP), 2 (100), 3 (300), 4 (600), 5 (1000)

**XP rewards**: experiment = 100, perfect quiz = 50, daily login = 10

### DIUAchievements

Soulbound (non-transferable) ERC-721 NFT badges.

| Function | Access | Description |
|----------|--------|-------------|
| `mint(user, achievement_id, uri)` | authorized | Mint achievement NFT |
| `has_achievement(user, id)` | view | Check if user earned badge |
| `get_achievements(user)` | view | List all user's tokens |
| `balance_of(owner)` | view | ERC-721 standard |
| `token_uri(token_id)` | view | ERC-721 metadata URI |
| `transfer_from(...)` | — | **Reverts** (soulbound) |

### DIUToken

ERC-20 platform token with restricted minting, public burning, and admin pause.

| Function | Access | Description |
|----------|--------|-------------|
| `mint(to, amount)` | authorized | Restricted minting (backend / cross-contract) |
| `burn(amount)` | token holder | Burn own tokens |
| `transfer(to, amount)` | token holder | ERC-20 standard |
| `approve(spender, amount)` | token holder | ERC-20 standard (infinite allowance supported) |
| `pause()` / `unpause()` | admin | Block all transfers, mints, burns |

## Security

| Pattern | Implementation |
|---------|----------------|
| Access Control | Owner → Admin → Authorized role hierarchy |
| Reentrancy | Disabled by default in Stylus (WASM execution model) |
| Overflow | Rust's checked arithmetic (`U256` operations) |
| Input Validation | Empty string checks, zero address checks |
| Soulbound NFTs | Transfer functions revert unconditionally |

**Security Advisor**: [Kirill Taran](https://github.com/diu-os) — Web3 Security

## Roadmap

### Phase 1: Foundation — **Complete** ✅

DIURegistry, DIUReputation, DIUAchievements, DIUToken → 4 contracts, 139 tests

### Phase 2: Extension (Apr–May 2026)

| Contract | Purpose |
|----------|---------|
| DIUProgress | Learning state, module completion tracking |
| DIUCrowdfunding | Research funding with milestones and refunds |

### Phase 3: DAO (2027+)

| Contract | Purpose |
|----------|---------|
| DIUGovernance | Proposals, voting, execution |
| DIUStaking | Token locking, voting power, rewards |

## Tech Stack

| Layer | Technology |
|-------|------------|
| Language | Rust |
| Framework | [Stylus SDK 0.10.0](https://github.com/OffchainLabs/stylus-sdk-rs) |
| Target | `wasm32-unknown-unknown` |
| Network | Arbitrum One (L2 Ethereum) |
| Testnet | Arbitrum Sepolia (Chain ID: 421614) |

## Project

DIU OS is part of a larger ecosystem:

| Repository | Description |
|------------|-------------|
| [diu-contracts](https://github.com/diu-os/diu-contracts) | Smart contracts (this repo) |
| [physics-tutorial](https://github.com/diu-os/physics-tutorial) | Interactive quantum physics MVP |
| [diu-os.github.io](https://github.com/diu-os/diu-os.github.io) | Landing page |
| [developer-portal](https://github.com/diu-os/developer-portal) | Developer documentation |
| [manifesto](https://github.com/diu-os/manifesto) | Project vision and principles |
| [diu-docs](https://github.com/diu-os/diu-docs) | General documentation |

**Live MVP**: [physics.diu-os.org](https://physics.diu-os.org)

## Testing on Arbitrum Sepolia

### Prerequisites

```bash
# foundry/cast for on-chain read/write calls
curl -L https://foundry.paradigm.xyz | bash
foundryup
```

Sepolia ETH для write-операций: https://faucet.triangleplatform.com/arbitrum/sepolia

### Deployed Contracts (19 Feb 2026)

| Contract | Address |
|----------|---------|
| DIURegistry | `0x49e1b11e1037e74113a7c0ccc41e3042d4691018` |
| DIUReputation | `0x8740f9d110133ff5efa0fb562e62ab92a466cdc5` |
| DIUAchievements | `0x1a9783ba7966c0e7299af7ee2228e19028d8ea7e` |
| DIUToken | `0xbbd9a558c049482f1be45399fec4a4c9dc1c810e` |

Deployer: `0x67bB4D1895D9A736F9e6076529B468ba05aeD150`

### Quick Verification (read-only)

```bash
export RPC="https://sepolia-rollup.arbitrum.io/rpc"
export REGISTRY="0x49e1b11e1037e74113a7c0ccc41e3042d4691018"
export REPUTATION="0x8740f9d110133ff5efa0fb562e62ab92a466cdc5"
export ACHIEVEMENTS="0x1a9783ba7966c0e7299af7ee2228e19028d8ea7e"
export TOKEN="0xbbd9a558c049482f1be45399fec4a4c9dc1c810e"

# DIURegistry — owner, total registered users
cast call $REGISTRY "owner()(address)" --rpc-url $RPC
cast call $REGISTRY "totalUsers()(uint256)" --rpc-url $RPC

# DIUReputation — owner
cast call $REPUTATION "owner()(address)" --rpc-url $RPC

# DIUAchievements — owner, total minted tokens
cast call $ACHIEVEMENTS "owner()(address)" --rpc-url $RPC

# DIUToken — name, symbol, total supply
cast call $TOKEN "name()(string)" --rpc-url $RPC
cast call $TOKEN "symbol()(string)" --rpc-url $RPC
cast call $TOKEN "totalSupply()(uint256)" --rpc-url $RPC
```

### Arbiscan Links

| Contract | Explorer |
|----------|---------|
| DIURegistry | https://sepolia.arbiscan.io/address/0x49e1b11e1037e74113a7c0ccc41e3042d4691018 |
| DIUReputation | https://sepolia.arbiscan.io/address/0x8740f9d110133ff5efa0fb562e62ab92a466cdc5 |
| DIUAchievements | https://sepolia.arbiscan.io/address/0x1a9783ba7966c0e7299af7ee2228e19028d8ea7e |
| DIUToken | https://sepolia.arbiscan.io/address/0xbbd9a558c049482f1be45399fec4a4c9dc1c810e |

### Running Local Tests

```bash
# All tests
cargo test

# Strict lint (zero warnings required)
cargo clippy -- -D warnings

# WASM check per contract (requires RPC)
cargo stylus check --features registry     --endpoint https://sepolia-rollup.arbitrum.io/rpc
cargo stylus check --features reputation   --endpoint https://sepolia-rollup.arbitrum.io/rpc
cargo stylus check --features achievements --endpoint https://sepolia-rollup.arbitrum.io/rpc
cargo stylus check --features token        --endpoint https://sepolia-rollup.arbitrum.io/rpc
```

## License

MIT
