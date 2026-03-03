#![cfg(feature = "all-contracts")]
//! Architectural Fitness Functions — DIU OS Smart Contracts
//!
//! Тестируют архитектурные инварианты (QARs из QAT.md), а не бизнес-логику.
//! Запуск: `cargo test --test fitness`
//!
//! NOTE: QAT.md указывает путь `src/tests/fitness.rs`, но `cargo test --test <name>`
//! работает только с integration-тестами в `tests/` (crate root). Правильный путь —
//! `diu-contracts/tests/fitness.rs`.
//!
//! Каждая fitness function помечена соответствующим QAR-ID из QAT.md.

use diu_contracts::registry::DIURegistry;
use diu_contracts::reputation::DIUReputation;
use diu_contracts::achievements::DIUAchievements;
use diu_contracts::token::DIUToken;
use diu_contracts::progress::DIUProgress;
use stylus_sdk::testing::*;
use alloy_primitives::{address, Address, U256};

const OWNER: Address = address!("1111111111111111111111111111111111111111");
const ATTACKER: Address = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
const ALICE: Address = address!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

// ═══════════════════════════════════════════════════════════════════════════
// ИНВАРИАНТ Q-08: Контракты не могут быть инициализированы дважды
// ═══════════════════════════════════════════════════════════════════════════

/// Двойная инициализация DIURegistry должна возвращать Err.
#[test]
fn ff_registry_cannot_be_reinitialized() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIURegistry::from(&vm);
    contract.initialize().expect("First init must succeed");

    let second = contract.initialize();
    assert!(
        second.is_err(),
        "INVARIANT VIOLATED (Q-08): DIURegistry re-initialization must be rejected"
    );
}

/// Двойная инициализация DIUReputation должна возвращать Err.
#[test]
fn ff_reputation_cannot_be_reinitialized() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUReputation::from(&vm);
    contract.initialize().expect("First init must succeed");

    let second = contract.initialize();
    assert!(
        second.is_err(),
        "INVARIANT VIOLATED (Q-08): DIUReputation re-initialization must be rejected"
    );
}

/// Двойная инициализация DIUAchievements должна возвращать Err.
#[test]
fn ff_achievements_cannot_be_reinitialized() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUAchievements::from(&vm);
    contract.initialize().expect("First init must succeed");

    let second = contract.initialize();
    assert!(
        second.is_err(),
        "INVARIANT VIOLATED (Q-08): DIUAchievements re-initialization must be rejected"
    );
}

/// Двойная инициализация DIUToken должна возвращать Err.
#[test]
fn ff_token_cannot_be_reinitialized() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUToken::from(&vm);
    contract.initialize().expect("First init must succeed");

    let second = contract.initialize();
    assert!(
        second.is_err(),
        "INVARIANT VIOLATED (Q-08): DIUToken re-initialization must be rejected"
    );
}

/// Двойная инициализация DIUProgress должна возвращать Err.
#[test]
fn ff_progress_cannot_be_reinitialized() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUProgress::from(&vm);
    contract
        .initialize(Address::ZERO, U256::from(10), U256::from(5))
        .expect("First init must succeed");

    let second = contract.initialize(Address::ZERO, U256::from(10), U256::from(5));
    assert!(
        second.is_err(),
        "INVARIANT VIOLATED (Q-08): DIUProgress re-initialization must be rejected"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ИНВАРИАНТ Q-05: Только authorized backend вызывает addXP
// ═══════════════════════════════════════════════════════════════════════════

/// add_xp с адреса-атакующего (не authorized) должен возвращать Err.
#[test]
fn ff_add_xp_rejects_unauthorized() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUReputation::from(&vm);
    contract.initialize().expect("Init must succeed");

    // Переключаемся на атакующего — он НЕ authorized
    vm.set_sender(ATTACKER);
    let result = contract.add_xp(ALICE, U256::from(100u64));
    assert!(
        result.is_err(),
        "INVARIANT VIOLATED (Q-05): Unauthorized addXP must be rejected"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ИНВАРИАНТ Q-06: Только authorized backend минтит токены
// ═══════════════════════════════════════════════════════════════════════════

/// mint с адреса-атакующего (не authorized) должен возвращать Err.
#[test]
fn ff_token_mint_rejects_unauthorized() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUToken::from(&vm);
    contract.initialize().expect("Init must succeed");

    vm.set_sender(ATTACKER);
    let result = contract.mint(ATTACKER, U256::from(1000u64));
    assert!(
        result.is_err(),
        "INVARIANT VIOLATED (Q-06): Unauthorized token mint must be rejected"
    );
}

/// mint badge с адреса-атакующего (не authorized) должен возвращать Err.
#[test]
fn ff_achievement_mint_rejects_unauthorized() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUAchievements::from(&vm);
    contract.initialize().expect("Init must succeed");

    vm.set_sender(ATTACKER);
    let result = contract.mint(
        ALICE,
        U256::from(1u64),
        String::from("ipfs://fake"),
    );
    assert!(
        result.is_err(),
        "INVARIANT VIOLATED (Q-06): Unauthorized achievement mint must be rejected"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ИНВАРИАНТ Q-16: Уровень (level) всегда в диапазоне 1–5
// ═══════════════════════════════════════════════════════════════════════════

/// get_level возвращает значение 1–5 для различных объёмов XP.
/// compute_level — приватная; тестируем косвенно через add_xp + get_level.
#[test]
fn ff_xp_level_within_safe_bounds() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUReputation::from(&vm);
    contract.initialize().expect("Init must succeed");

    // Проверяем уровень без XP (свежий пользователь)
    let level = contract.get_level(ALICE);
    assert!(
        (1..=5).contains(&level),
        "INVARIANT VIOLATED (Q-16): Level must be 1-5, got {} for 0 xp",
        level
    );

    // Граничные значения XP: добавляем разные суммы и проверяем уровень
    // Таблица уровней: 1(0), 2(100), 3(300), 4(600), 5(1000)
    let xp_values = [
        U256::from(1u64),
        U256::from(100u64),
        U256::from(300u64),
        U256::from(600u64),
        U256::from(1000u64),
        U256::from(50_000u64),
    ];

    for &xp in &xp_values {
        let vm_inner = TestVMBuilder::new().sender(OWNER).build();
        let mut c = DIUReputation::from(&vm_inner);
        c.initialize().expect("Init must succeed");
        // OWNER is authorized after initialize
        c.add_xp(ALICE, xp).expect("add_xp should succeed");
        let lvl = c.get_level(ALICE);
        assert!(
            (1..=5).contains(&lvl),
            "INVARIANT VIOLATED (Q-16): Level must be 1-5, got {} for xp={}",
            lvl,
            xp
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ИНВАРИАНТ Q-18: DIUProgress record_simulation — только authorized backend
// ═══════════════════════════════════════════════════════════════════════════

/// record_simulation с адреса-атакующего должен возвращать Err.
#[test]
fn ff_progress_record_rejects_unauthorized() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUProgress::from(&vm);
    contract
        .initialize(Address::ZERO, U256::from(10), U256::from(5))
        .expect("Init must succeed");

    vm.set_sender(ATTACKER);
    let result = contract.record_simulation(ALICE, U256::from(0u64), U256::from(50u64));
    assert!(
        result.is_err(),
        "INVARIANT VIOLATED (Q-18): Unauthorized record_simulation must be rejected"
    );
}

/// record_module_completion с адреса-атакующего должен возвращать Err.
#[test]
fn ff_progress_module_rejects_unauthorized() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUProgress::from(&vm);
    contract
        .initialize(Address::ZERO, U256::from(10), U256::from(5))
        .expect("Init must succeed");

    vm.set_sender(ATTACKER);
    let result = contract.record_module_completion(ALICE, U256::from(0u64));
    assert!(
        result.is_err(),
        "INVARIANT VIOLATED (Q-18): Unauthorized record_module_completion must be rejected"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ИНВАРИАНТ Q-06 (badges): Двойной mint badge отклоняется
// ═══════════════════════════════════════════════════════════════════════════

/// Повторный mint того же achievement_id одному пользователю должен возвращать Err.
#[test]
fn ff_achievement_double_mint_rejected() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUAchievements::from(&vm);
    contract.initialize().expect("Init must succeed");

    // OWNER is authorized after initialize
    contract
        .mint(ALICE, U256::from(1u64), String::from("ipfs://badge1"))
        .expect("First mint must succeed");

    let second = contract.mint(ALICE, U256::from(1u64), String::from("ipfs://badge1-dup"));
    assert!(
        second.is_err(),
        "INVARIANT VIOLATED (Q-06): Double mint of same badge must be rejected"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ИНВАРИАНТ: Soulbound NFT — трансфер всегда запрещён
// ═══════════════════════════════════════════════════════════════════════════

/// transfer_from для soulbound badge всегда должен возвращать Err.
#[test]
fn ff_achievement_transfer_always_rejected() {
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut contract = DIUAchievements::from(&vm);
    contract.initialize().expect("Init must succeed");

    let token_id = contract
        .mint(ALICE, U256::from(1u64), String::from("ipfs://badge"))
        .expect("Mint must succeed");

    // Даже owner не может трансферить soulbound badge
    let result = contract.transfer_from(ALICE, ATTACKER, token_id);
    assert!(
        result.is_err(),
        "INVARIANT VIOLATED: Soulbound transfer_from must always revert"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ИНВАРИАНТ Q-14: Изоляция контрактов — ошибка одного не влияет на другие
// ═══════════════════════════════════════════════════════════════════════════

/// Каждый контракт имеет независимое состояние. Ошибка в одном не затрагивает другой.
#[test]
fn ff_contracts_state_is_isolated() {
    // Создаём DIUReputation, вызываем невалидную операцию (→ Err)
    let vm1 = TestVMBuilder::new().sender(ATTACKER).build();
    let mut rep = DIUReputation::from(&vm1);
    // ATTACKER не может инициализировать и делать что-либо полезное,
    // но initialize сработает (любой может вызвать initialize первым)
    rep.initialize().expect("Init must succeed");
    // Пытаемся добавить XP с другого адреса — Err
    vm1.set_sender(Address::ZERO);
    let failed = rep.add_xp(ALICE, U256::from(100u64));
    assert!(failed.is_err(), "Precondition: zero-address add_xp should fail");

    // После ошибки в Reputation — Registry должен работать нормально
    let vm2 = TestVMBuilder::new().sender(OWNER).build();
    let mut registry = DIURegistry::from(&vm2);
    registry
        .initialize()
        .expect("INVARIANT VIOLATED (Q-14): Registry init failed after Reputation error");

    // Registry полностью функционален
    registry
        .register_user(String::from("ipfs://user1"))
        .expect("INVARIANT VIOLATED (Q-14): Registry register failed after Reputation error");
    assert!(
        registry.is_registered(OWNER),
        "INVARIANT VIOLATED (Q-14): Contract state isolation broken"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// ИНВАРИАНТ: Owner-only операции отклоняют не-owner
// ═══════════════════════════════════════════════════════════════════════════

/// grant_authorized от не-owner должен возвращать Err (все контракты с backend ролью).
#[test]
fn ff_grant_authorized_rejects_non_owner() {
    // DIUReputation
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut rep = DIUReputation::from(&vm);
    rep.initialize().expect("Init must succeed");
    vm.set_sender(ATTACKER);
    assert!(
        rep.grant_authorized(ATTACKER).is_err(),
        "INVARIANT VIOLATED: Non-owner grant_authorized on Reputation must be rejected"
    );

    // DIUToken
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut token = DIUToken::from(&vm);
    token.initialize().expect("Init must succeed");
    vm.set_sender(ATTACKER);
    assert!(
        token.grant_authorized(ATTACKER).is_err(),
        "INVARIANT VIOLATED: Non-owner grant_authorized on Token must be rejected"
    );

    // DIUAchievements
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut ach = DIUAchievements::from(&vm);
    ach.initialize().expect("Init must succeed");
    vm.set_sender(ATTACKER);
    assert!(
        ach.grant_authorized(ATTACKER).is_err(),
        "INVARIANT VIOLATED: Non-owner grant_authorized on Achievements must be rejected"
    );

    // DIUProgress
    let vm = TestVMBuilder::new().sender(OWNER).build();
    let mut prog = DIUProgress::from(&vm);
    prog.initialize(Address::ZERO, U256::from(10), U256::from(5))
        .expect("Init must succeed");
    vm.set_sender(ATTACKER);
    assert!(
        prog.grant_authorized(ATTACKER).is_err(),
        "INVARIANT VIOLATED: Non-owner grant_authorized on Progress must be rejected"
    );
}
