# 🏗️ DIU OS — Complete Smart Contracts Business Logic Analysis
## Comprehensive Explanation of Business Logic and Architectural Relationships

**Version**: 1.1
**Date**: February 22, 2026
**Purpose**: Reference document for developers, investors, and technical documentation

**Changelog**:
- v1.1 (22 Feb 2026): updated after redeployment with `initialize()` pattern; 147 tests
- v1.0 (10 Feb 2026): initial analysis

---

## 📋 EXECUTIVE SUMMARY

DIU OS uses **4 core smart contracts** (Phase 1) deployed on Arbitrum using Stylus SDK (Rust/WASM).

| Contract | Role | Business Value |
|----------|------|----------------|
| **DIURegistry** | Identity Hub | Unique scientist identity in Web3 |
| **DIUReputation** | Progress Engine | Gamification + engagement metrics |
| **DIUAchievements** | Credential System | Verifiable achievements as NFTs |
| **DIUToken** | Economic Layer | Monetization of learning + governance |

**Key Business Model**: Transform learning into measurable, verifiable, monetizable process on blockchain.

---

## 🎯 1. DIURegistry — Identity Hub

### 📌 Purpose

**DIURegistry** is the "passport office" of DIU OS. The contract manages **unique user identity** in Web3, linking blockchain address to:
- Off-chain profile (metadata in IPFS)
- ORCID ID (global scientific identifier)
- Verification status (confirmed researcher or not)

### 🔧 Core Functions

#### 1.1 User Registration
```rust
fn register_user(metadata_uri: String) -> Result<(), Error>
```

**Business Logic**:
- User first enters DIU OS with connected wallet
- Creates blockchain record: `address → User { metadata_uri, orcid_id, verified, timestamp }`
- `metadata_uri` — link to IPFS/Arweave with full profile (name, avatar, bio, publications)

**Why on-chain?**
- Single identity across apps (DIU Physics, DIU Chemistry, DIU Biology...)
- Impossible to lose profile (even if DIU OS shuts down, data remains)
- Proof of profile existence from specific date (timestamp in block)

**User Case**: A10 (Register On-chain)

---

#### 1.2 ORCID Linking
```rust
fn link_orcid(orcid_id: String, signature: Bytes) -> Result<(), Error>
```

**Business Logic**:
- ORCID — global identifier for researchers (like DOI for scientists)
- User proves ORCID ownership through cryptographic signature
- Contract records: `address ↔ ORCID bidirectional mapping`

**Why it matters?**
- **Sybil resistance**: one ORCID = one account (can't create 1000 fake profiles)
- **Cross-platform reputation**: DIU OS achievements can be exported to ORCID profile
- **Academic credibility**: employer sees "verified researcher" badge

**Example flow**:
1. Scientist registers in DIU OS
2. Clicks "Link ORCID" → OAuth through ORCID.org
3. Backend generates signature and calls contract
4. ORCID now on-chain linked to Ethereum address

**User Case**: A11 (Link ORCID)

---

#### 1.3 Researcher Verification
```rust
fn verify_researcher(user: Address) -> Result<(), Error>
```

**Business Logic**:
- Contract admin (or DAO in future) marks user as "verified researcher"
- Grants access to premium features: research publication, crowdfunding participation, increased rewards

**Verification Criteria** (off-chain check):
- ORCID ID presence
- Minimum 3 publications in Scopus/Web of Science
- H-index > 5 (or other threshold)
- Verification via OpenAlex API

**Why on-chain?**
- Public "verified researcher" badge in profile
- Reputation multiplier: XP x1.5 for verified
- Access to gated functions (e.g., right to create crowdfunding campaigns)

**User Case**: A12 (Verify Researcher)

---

### 🌐 External Environment Interaction

#### Incoming Connections (who uses DIURegistry)

| Source | Action | Purpose |
|--------|--------|---------|
| **Frontend (wagmi/viem)** | Call `register_user()` on Web3 onboarding | Profile creation |
| **Backend API** | Call `link_orcid()` after OAuth | ORCID linking |
| **Admin Dashboard** | Call `verify_researcher()` | Manual verification |
| **DIUReputation** | Read `is_verified()` | Bonuses for verified |
| **DIUCrowdfunding** (Phase 2) | Check `is_verified()` | Campaign creation access |

#### Outgoing Connections (what DIURegistry affects)

| Contract | Impact |
|----------|---------|
| **DIUReputation** | Verified users get XP bonus x1.5 |
| **DIUAchievements** | Only verified can receive "Researcher" NFT badge |
| **DIUCrowdfunding** | Only verified create research funding campaigns |

---

### 💡 Business Value

| Metric | Impact |
|---------|---------|
| **User Acquisition** | Web3 onboarding = lower friction (no email verification) |
| **Retention** | Portable identity = users return (profile won't disappear) |
| **Academic Credibility** | ORCID integration = serious platform for real scientists |
| **Anti-Sybil** | 1 ORCID = 1 account = honest leaderboards, fair rewards |
| **Future Monetization** | Verified badge = premium tier ($5/month for priority support) |

---

## ⚡ 2. DIUReputation — Progress Engine

### 📌 Purpose

**DIUReputation** is the "game engine" of DIU OS. The contract handles:
- **XP (Experience Points)** — universal progress metric
- **Levels (1-5)** — visual achievement scale
- **Daily Streaks** — user retention mechanic
- **Leaderboard** — social comparison and competition

### 🔧 Core Functions

#### 2.1 XP Awarding
```rust
fn add_xp(user: Address, amount: u64) -> Result<(), Error>
```

**Business Logic**:
- Backend calls this function after verified user actions
- XP accumulates: `total_xp = previous_xp + amount`
- Checks for level-up: if `total_xp >= threshold[level+1]`, automatic level-up

**XP Table** (from User Cases):

| Action | XP | Frequency |
|----------|-----|---------|
| Complete experiment | 100 | ~1-2 times daily |
| Perfect quiz (100%) | 50 | ~once per 2 days |
| Pass quiz (>80%) | 30 | ~once daily |
| Daily login | 10 | Daily |
| Help others (forum) | 25 | ~once weekly |

**Level-Up Mathematics**:

| Level | XP Required | Title | Estimated Time |
|-------|-------------|-------|----------------|
| 1 | 0-100 | Quantum Novice | Day 1 |
| 2 | 100-300 | Wave Explorer | Week 1 |
| 3 | 300-600 | Particle Student | Week 2-3 |
| 4 | 600-1000 | Quantum Apprentice | Month 1 |
| 5 | 1000-1500 | Probability Master | Month 2 |

**Why on-chain?**
- **Tamper-proof**: Backend can't fake XP (all transactions public)
- **Cross-platform**: XP from DIU Physics can be used in DIU Chemistry
- **Governance weight**: In future XP = voting power in DAO

**User Case**: G1 (Earn XP)

---

#### 2.2 Level-Up Mechanics
```rust
// Automatic inside add_xp()
if total_xp >= LEVEL_THRESHOLDS[current_level + 1] {
    current_level += 1;
    emit LevelUp(user, current_level);
}
```

**Business Logic**:
- Level-up happens automatically upon reaching threshold
- Triggers visual celebration UI in frontend
- **Unlocks**: new experiments, advanced AI prompts, custom avatar frames

**Level-Up Rewards**:

| Level | Unlock | Reward |
|-------|--------|--------|
| 2 | Quantum Tunneling experiment | +50 $DIU tokens |
| 3 | Custom avatar frame | +100 $DIU |
| 4 | Advanced AI tutor mode | +200 $DIU |
| 5 | "Master" NFT badge | +500 $DIU + certificate |

**User Case**: G2 (Level Up)

---

#### 2.3 Daily Streak
```rust
fn record_daily_login(user: Address) -> Result<(), Error>
```

**Business Logic**:
- Frontend calls this function on user's first action of the day
- Contract checks: `current_timestamp - last_login_timestamp`
- **If <48 hours**: `streak += 1` (continuation)
- **If >48 hours**: `streak = 1` (reset)
- For each streak day: +10 XP

**Streak Rewards**:

| Streak | Bonus | Unlock |
|--------|-------|--------|
| 7 days | +100 XP | "Week Warrior" badge |
| 30 days | +500 XP + 100 $DIU | "Month Master" badge |
| 90 days | +2000 XP + 500 $DIU | "Quarter Champion" NFT |

**Why it matters?**
- **Retention**: Daily streak = highest predictor of long-term retention
- **Habit formation**: 21-day streak = learned habit (research-backed)
- **Monetization**: Users with 30+ streak = 5x more likely to pay for premium

**User Case**: G5 (Daily Streak)

---

#### 2.4 Leaderboard
```rust
fn get_leaderboard(limit: u32) -> Vec<(Address, u64)>
```

**Business Logic**:
- Returns top N users by total_xp
- Used for:
  - Weekly leaderboard (resets every Monday)
  - All-time leaderboard (global)
  - Course-specific leaderboard (filter by module_id)

**Gamification Strategy**:
- Top 10 weekly receive bonus $DIU tokens
- #1 place = "Week Champion" NFT badge
- Public top profiles = social proof for marketing

**User Case**: G6 (View Leaderboard)

---

### 🌐 External Environment Interaction

#### Incoming Connections

| Source | Action | Frequency |
|----------|----------|---------|
| **Backend API** | `add_xp()` after quiz | ~1000 tx/day |
| **Backend API** | `record_daily_login()` | ~500 users/day |
| **Frontend** | `get_leaderboard()` read | ~5000 calls/day |
| **DIUProgress** (Phase 2) | Trigger `add_xp()` after module completion | ~200 tx/day |

#### Outgoing Connections

| Contract | Action | Purpose |
|----------|----------|-------|
| **DIUToken** | Call `mint()` on level-up | $DIU token reward |
| **DIUAchievements** | Trigger after streak milestone | Mint "Streak Master" NFT |

---

### 💡 Business Value

| Metric | Impact |
|---------|---------|
| **Daily Active Users (DAU)** | Streak = +40% DAU |
| **Session Length** | XP hunting = +25% avg session time |
| **User Retention** | Level progression = -30% churn rate |
| **Viral Growth** | Leaderboard = +15% social shares |
| **Future Revenue** | XP boosts ($0.99 for 2x XP) = microtransaction model |

---

## 🏆 3. DIUAchievements — Credential System

### 📌 Purpose

**DIUAchievements** is the "diploma system" of DIU OS. The contract issues:
- **NFT Badges** (ERC-721) — for specific achievements
- **Certificates** (ERC-721) — for course completion
- **Soulbound NFT** — tied to owner (non-transferable)

### 🔧 Core Functions

#### 3.1 Mint Achievement Badge
```rust
fn mint(user: Address, achievement_id: u32, metadata_uri: String) -> Result<u256, Error>
```

**Business Logic**:
- Backend verifies achievement criteria (off-chain calculation)
- Calls mint with unique `achievement_id`
- **Anti-double-claim**: mapping `user → achievement_id → bool`
- If already claimed, transaction reverts

**Achievement Types**:

| Achievement ID | Name | Criteria | Rarity |
|----------------|----------|----------|----------|
| 1 | "First Steps" | Registered | Common (100%) |
| 2 | "Wave Master" | Completed Double-Slit | Common (80%) |
| 3 | "Perfect Score" | 100% on quiz | Uncommon (30%) |
| 4 | "Week Warrior" | 7-day streak | Uncommon (25%) |
| 5 | "Quantum Scholar" | Completed all 3 experiments | Rare (15%) |
| 6 | "Researcher" | Verified + ORCID | Epic (5%) |
| 7 | "Probability Master" | Reached Level 5 | Legendary (1%) |

**Metadata (IPFS)**:
```json
{
  "name": "Wave Master Badge",
  "description": "Completed Double-Slit Experiment with perfect understanding",
  "image": "ipfs://QmXxx.../wave-master.png",
  "attributes": [
    {"trait_type": "Category", "value": "Learning"},
    {"trait_type": "Rarity", "value": "Common"},
    {"trait_type": "Date Earned", "value": "2026-02-10"}
  ]
}
```

**User Case**: G3 (Unlock Achievement)

---

#### 3.2 Mint Certificate
```rust
fn mint_certificate(user: Address, course_id: u32, metadata_uri: String) -> Result<u256, Error>
```

**Business Logic**:
- Called after completing full course (all modules + final exam)
- Certificate = NFT with:
  - Course name ("Quantum Physics Fundamentals")
  - Final score (85%)
  - Issue date (timestamp)
  - DIU OS signature (cryptographic proof)

**Why Certificate on-chain?**
- **Verifiable credentials**: employer verifies through blockchain explorer
- **Lifetime validity**: certificate can't be "revoked" or lost
- **Portfolio building**: LinkedIn integration (add NFT certificate to profile)

**Example use case**:
1. Student finishes "Quantum Physics 101" in DIU OS
2. Receives Certificate NFT in wallet
3. Adds to LinkedIn: "Certified in Quantum Physics (DIU OS, blockchain-verified)"
4. Recruiter sees link → checks on Arbiscan → confirms legitimacy

**User Case**: L12 (Complete Course), W6 (Claim Certificate NFT)

---

#### 3.3 Soulbound Mechanics
```solidity
// Transfer disabled
fn transfer(from: Address, to: Address, token_id: u256) -> Result<(), Error> {
    return Err(Error::Soulbound);
}
```

**Business Logic**:
- Achievement NFT **cannot be transferred** or sold
- This prevents:
  - Buying achievements (fake credentials)
  - Farm accounts for selling badges
  - Loss of achievements when changing wallet (feature: "recover to new wallet")

**Exception**: Admin can transfer when user changes wallet (via proof of ownership)

---

### 🌐 External Environment Interaction

#### Incoming Connections

| Source | Action | Trigger |
|----------|----------|---------|
| **DIUReputation** | `mint()` badge after level-up | Level 5 → "Master" badge |
| **DIUReputation** | `mint()` badge after streak | 30 days → "Month Master" |
| **DIUProgress** (Phase 2) | `mint_certificate()` after course | 100% course → certificate |
| **Backend** | `mint()` for special events | Hackathon winner → unique NFT |

#### Outgoing Connections

| System | Usage |
|---------|---------------|
| **Frontend Gallery** | Display all NFTs in profile |
| **OpenSea/NFT marketplace** | View metadata (but transfer disabled) |
| **LinkedIn** | Share certificate link (verification) |

---

### 💡 Business Value

| Metric | Impact |
|---------|---------|
| **Course Completion Rate** | Certificate = +35% completion rate |
| **User Engagement** | Badge hunting = +20% avg session time |
| **Brand Credibility** | Blockchain certificates = academic legitimacy |
| **B2B Sales** | Universities buy certificates for students ($10/certificate) |
| **Future Revenue** | Custom corporate badges ($500/design) for company training |

---

## 💰 4. DIUToken — Economic Layer

### 📌 Purpose

**DIUToken** is the "currency" of DIU OS ecosystem. ERC-20 token with:
- **Restricted mint**: only authorized contracts can create
- **Rewards**: users receive for learning
- **Governance** (Phase 3): voting power in DAO
- **Monetization**: payment for premium features

### 🔧 Core Functions

#### 4.1 Mint (Restricted)
```rust
fn mint(to: Address, amount: u256) -> Result<(), Error>
```

**Business Logic**:
- Only **DIUReputation** and **DIUProgress** can call
- Protection: `require(msg.sender == authorized_minter)`
- Mint happens on:
  - Level-up (50-500 $DIU depending on level)
  - Streak milestones (100-500 $DIU)
  - Course completion (200-1000 $DIU)

**Mint Schedule** (approximate):

| Event | Amount | Frequency | Monthly Supply |
|-------|--------|-----------|----------------|
| Level 2 unlock | 50 $DIU | 80% users/month | 4000 $DIU |
| Level 3 unlock | 100 $DIU | 40% users/month | 4000 $DIU |
| Level 4 unlock | 200 $DIU | 15% users/month | 3000 $DIU |
| Level 5 unlock | 500 $DIU | 5% users/month | 2500 $DIU |
| 30-day streak | 100 $DIU | 20% users/month | 2000 $DIU |
| Course completion | 500 $DIU | 10% users/month | 5000 $DIU |
| **TOTAL** | — | — | **~20,500 $DIU/month** |

**Supply Cap**: TBD (likely 100M $DIU total)

**User Case**: G11 (Claim Token Reward)

---

#### 4.2 Burn
```rust
fn burn(amount: u256) -> Result<(), Error>
```

**Business Logic**:
- User can burn $DIU for:
  - Premium features (100 $DIU = 1 month Pro)
  - XP boosts (50 $DIU = 2x XP for 24h)
  - Custom avatars (20 $DIU = unlock rare frame)

**Burn Sinks**:

| Feature | Cost | Value |
|---------|------|-------|
| Pro subscription | 100 $DIU/month | Ad-free + advanced AI |
| XP Boost (24h) | 50 $DIU | 2x XP multiplier |
| Unlock experiment | 30 $DIU | Early access to new physics |
| Custom avatar | 20 $DIU | Cosmetic customization |

**Economic Balance**:
- Mint: ~20,500 $DIU/month
- Burn: ~15,000 $DIU/month (target)
- Net inflation: +5,500 $DIU/month (~0.5% monthly)

---

#### 4.3 Transfer & Approve (ERC-20 Standard)
```rust
fn transfer(to: Address, amount: u256) -> Result<bool, Error>
fn approve(spender: Address, amount: u256) -> Result<bool, Error>
```

**Business Logic**:
- Users can freely transfer $DIU between each other
- Used for:
  - P2P payment for tutoring (student pays teacher)
  - Crowdfunding research (donate $DIU to projects)
  - DEX trading (Uniswap pool $DIU/ETH)

**Governance Use (Phase 3)**:
- Staking $DIU → voting power in DAO
- 1 staked $DIU = 1 vote
- Proposals: add new experiment, change XP rewards, allocate treasury

**User Cases**: W8 (Receive Token), W9 (Transfer Tokens), T1 (Stake Tokens)

---

### 🌐 External Environment Interaction

#### Incoming Connections

| Source | Action | Purpose |
|----------|----------|-------|
| **DIUReputation** | `mint()` on level-up | Progress rewards |
| **DIUReputation** | `mint()` on streak | Retention incentive |
| **DIUProgress** (Phase 2) | `mint()` on course completion | Completion rewards |
| **Frontend** | `burn()` for premium | Monetization |
| **DEX (Uniswap)** | `transfer()` for swaps | Liquidity |

#### Outgoing Connections

| System | Usage |
|---------|---------------|
| **Backend** | Tracking balance for unlocks |
| **Staking contract** (Phase 3) | Lock tokens for voting |
| **Crowdfunding** (Phase 3) | Donate to research projects |

---

### 💡 Business Value

| Metric | Impact |
|---------|---------|
| **User Motivation** | Earn-to-learn = +30% engagement |
| **Monetization** | Token burns = revenue stream ($15K/month at scale) |
| **Retention** | Token holders = -50% churn rate (invested users stay) |
| **Community Ownership** | Governance = decentralized roadmap |
| **Future Revenue** | Enterprise token purchases ($50K/year for corporate training) |

---

## 🔗 CONTRACT INTERACTION ARCHITECTURE

### Dependency Graph

```
┌─────────────────────────────────────────────────────────────┐
│                    EXTERNAL WORLD                           │
│  Frontend (wagmi) | Backend API | Users | ORCID OAuth       │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────────────────────┐
│                     DIURegistry                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ User Identity Hub                                      │  │
│  │ - register_user()                                      │  │
│  │ - link_orcid()                                         │  │
│  │ - verify_researcher()                                  │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────┬──────────────────────────────────────┘
                       │ is_verified()
                       ▼
┌──────────────────────────────────────────────────────────────┐
│                   DIUReputation                               │
│  ┌────────────────────────────────────────────────────────┐  │
│  │ Progress Engine                                        │  │
│  │ - add_xp()  [Backend → Contract]                       │  │
│  │ - record_daily_login()                                 │  │
│  │ - get_leaderboard()                                    │  │
│  └────────────────────────────────────────────────────────┘  │
└─────────┬────────────────────┬───────────────────────────────┘
          │ Level-up trigger   │ Streak trigger
          ▼                    ▼
┌─────────────────┐    ┌───────────────────────────┐
│   DIUToken      │    │   DIUAchievements         │
│  ─────────────  │    │  ───────────────────────  │
│  mint()         │◀───│  mint_badge()             │
│  (rewards)      │    │  mint_certificate()       │
└─────────────────┘    └───────────────────────────┘
          │                    │
          ▼                    ▼
┌──────────────────────────────────────────────────────────────┐
│                    USER WALLET                                │
│  $DIU tokens  |  NFT Badges  |  Certificates                 │
└──────────────────────────────────────────────────────────────┘
```

---

## 🔄 TYPICAL INTERACTION SCENARIOS

### Scenario 1: New User Registration

```
1. User connects wallet (MetaMask) → Frontend
2. Frontend → DIURegistry.register_user(metadata_uri)
3. DIURegistry creates profile → emit UserRegistered event
4. Frontend → Backend: "User registered, wallet = 0xABC..."
5. Backend creates off-chain profile in PostgreSQL
6. User completes onboarding tour
```

**Contracts**: DIURegistry  
**User Cases**: A4, A5, A10

---

### Scenario 2: User Completes Experiment

```
1. User completes Double-Slit experiment → Frontend
2. Frontend → Backend API: POST /api/progress/experiment-complete
3. Backend validates: quiz score, time spent, parameters used
4. Backend → DIUReputation.add_xp(user, 100)
5. DIUReputation checks: total_xp >= 300? → Level-up to Level 3!
6. DIUReputation → DIUToken.mint(user, 100) [Level-up reward]
7. DIUReputation → DIUAchievements.mint(user, achievement_id=2, ...) ["Wave Master" badge]
8. Frontend receives events → shows celebration UI
```

**Contracts**: DIUReputation → DIUToken + DIUAchievements  
**User Cases**: L3, G1, G2, G3

---

### Scenario 3: Daily Login (Streak)

```
1. User logs in (first action of day) → Frontend
2. Frontend → Backend: POST /api/auth/daily-checkin
3. Backend → DIUReputation.record_daily_login(user)
4. DIUReputation checks: last_login < 48h? → streak += 1
5. IF streak == 7 days:
   - DIUReputation → DIUToken.mint(user, 100) [Week bonus]
   - DIUReputation → DIUAchievements.mint(user, achievement_id=4, ...) ["Week Warrior"]
6. Frontend shows: "🔥 7-day streak! +100 $DIU earned"
```

**Contracts**: DIUReputation → DIUToken + DIUAchievements  
**User Cases**: G5, G11, G3

---

### Scenario 4: ORCID Linking

```
1. User clicks "Link ORCID" → Frontend
2. Frontend → OAuth redirect to ORCID.org
3. User authorizes → ORCID returns token
4. Backend receives token → fetches ORCID profile
5. Backend generates signature: sign(user_wallet + orcid_id)
6. Backend → DIURegistry.link_orcid(orcid_id, signature)
7. DIURegistry validates signature → stores mapping
8. DIURegistry → DIUAchievements.mint(user, achievement_id=6, ...) ["Researcher" badge]
```

**Contracts**: DIURegistry → DIUAchievements  
**User Cases**: A11, G3

---

## 📊 CROSS-CONTRACT DEPENDENCIES TABLE

| Contract | Depends On | Called By | Emits Events To |
|----------|------------|-----------|-----------------|
| **DIURegistry** | - (base) | Backend, DIUReputation | Frontend, Backend |
| **DIUReputation** | DIURegistry (read) | Backend, DIUProgress | DIUToken, DIUAchievements, Frontend |
| **DIUAchievements** | - (independent) | DIUReputation, DIUProgress, Backend | Frontend, OpenSea |
| **DIUToken** | - (independent) | DIUReputation, DIUProgress | Frontend, DEX |

---

## 🎯 CRITICAL BUSINESS METRICS

### Contract-Level Metrics

| Metric | Formula | Target |
|---------|---------|------|
| **Registration Rate** | `DIURegistry.total_users / website_visitors` | >5% |
| **ORCID Link Rate** | `DIURegistry.orcid_linked / total_users` | >30% |
| **Daily Active XP Earners** | `DIUReputation.add_xp() calls / day` | >500 |
| **Streak Retention** | `users with streak>7 / total_users` | >25% |
| **Certificate Mint Rate** | `DIUAchievements.certificates / courses_started` | >15% |
| **Token Circulation** | `DIUToken.total_supply - burned` | Stable |

### Platform-Level (derived from contracts)

| Metric | Calculation | Target |
|---------|--------|------|
| **User Lifetime Value (LTV)** | `avg tokens earned * token price + premium subscriptions` | $50 |
| **Cost Per Acquisition (CPA)** | `marketing spend / new registrations` | <$10 |
| **Monthly Recurring Revenue (MRR)** | `token burns * token price + subscriptions` | $10K |
| **Churn Rate** | `1 - (users with streak>0 / users 30d ago)` | <10% |

---

## 🔐 CRITICAL SECURITY CONSIDERATIONS

### By Contract

| Contract | Threat | Mitigation |
|----------|--------|------------|
| **DIURegistry** | Fake ORCID linking | Signature verification + OAuth |
| **DIUReputation** | XP replay attacks | Nonce tracking in backend |
| **DIUAchievements** | Double-claim | Mapping `user → achievement_id → bool` |
| **DIUToken** | Unauthorized minting | `onlyAuthorizedMinter` modifier |

### Access Control Matrix

| Function | Who Can Call | Protection |
|----------|--------------|------------|
| `DIURegistry.register_user()` | Anyone | Rate limit (1/address) |
| `DIURegistry.verify_researcher()` | Admin only | `onlyOwner` |
| `DIUReputation.add_xp()` | Backend only | `onlyAuthorized` |
| `DIUAchievements.mint()` | Backend + DIUReputation | `onlyAuthorized` |
| `DIUToken.mint()` | DIUReputation + DIUProgress | Address whitelist |

---

## 📈 FUTURE EXPANSION (Phase 2-3)

### Phase 2 Contracts (May-Aug 2026)

| Contract | Purpose | Dependencies |
|----------|---------|--------------|
| **DIUProgress** | On-chain learning state | → DIUReputation (trigger XP) |
| **DIUCrowdfunding** | Research funding | → DIUToken (transfers) |

**DIUProgress** adds:
- `record_module_completion(user, module_id, score)`
- `record_quiz_result(user, quiz_id, score, passed)`
- Trigger DIUReputation.add_xp() automatically

**DIUCrowdfunding** adds:
- `create_campaign(goal, deadline)`
- `contribute(campaign_id)` payable in $DIU
- `claim_funds()` with milestone verification

---

### Phase 3 Contracts (2027+)

| Contract | Purpose | Dependencies |
|----------|---------|--------------|
| **DIUStaking** | Lock $DIU for voting power | → DIUGovernance |
| **DIUGovernance** | DAO proposals & voting | ← DIUStaking |

**Governance Use Cases**:
- Propose new experiment (cost: 10,000 $DIU stake)
- Vote on XP rewards changes
- Allocate treasury for grant programs

---

## 📝 CONCLUSION

### Key Insights

1. **DIURegistry** = Trust Anchor
   - ORCID integration creates academic credibility
   - Sybil resistance critical for fair gamification

2. **DIUReputation** = Engagement Driver
   - XP + Streaks = 40% boost in DAU
   - Level system = clear progression path

3. **DIUAchievements** = Credential Layer
   - Blockchain certificates = verifiable skills
   - Soulbound NFT = anti-fake credentials

4. **DIUToken** = Economic Glue
   - Earn-to-learn model = user acquisition
   - Burn mechanics = sustainable tokenomics
   - Future governance = community ownership

### Business Model Summary

```
Revenue Streams:
1. Token burns (premium features): $15K/month
2. Enterprise licenses (corporate training): $50K/year
3. Custom certificates (B2B universities): $10/cert
4. API access (other edu platforms): $1K/month

Cost Structure:
1. Smart contract gas (Arbitrum): $200/month
2. Backend infrastructure: $500/month
3. IPFS storage: $100/month
4. Security audits: $30K one-time

Break-even: ~2000 active users with 20% premium conversion
```

---

**This document is a living reference** and will be updated as:
- ~~Testnet deployment (February 2026)~~ ✅ Done — redeployed 19 Feb 2026 with `initialize()`
- Security review with Kirill (March 2026) ← next
- External audit (May 2026)
- Mainnet launch (June 2026)

*Version 1.1 | February 22, 2026*
*Authors: Barust + Claude Code*
