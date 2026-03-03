#!/usr/bin/env bash
# DIU OS — Security Check Script
# Тестирует access control для всех 5 контрактов на Arbitrum Sepolia.
# Запуск: ATTACKER_KEY=<test-key> bash scripts/security-check.sh
#
# ВНИМАНИЕ: ATTACKER_KEY — тестовый ключ, НЕ deployer!
# Никогда не используй deployer key как ATTACKER_KEY.
#
# Требования: cast (из foundry), установить: curl -L https://foundry.paradigm.xyz | bash

set -euo pipefail

RPC="https://sepolia-rollup.arbitrum.io/rpc"

REGISTRY="0x49e1b11e1037e74113a7c0ccc41e3042d4691018"
REPUTATION="0x8740f9d110133ff5efa0fb562e62ab92a466cdc5"
ACHIEVEMENTS="0x1a9783ba7966c0e7299af7ee2228e19028d8ea7e"
TOKEN="0xbbd9a558c049482f1be45399fec4a4c9dc1c810e"
PROGRESS="0xb1c4edc73aae322f62cda57f84f303761ca3e347"

DUMMY_ADDR="0x1234567890123456789012345678901234567890"

if [[ -z "${ATTACKER_KEY:-}" ]]; then
  echo "ERROR: ATTACKER_KEY env var required (тестовый ключ, НЕ deployer)"
  exit 1
fi

pass=0
fail=0

check() {
  local label="$1"
  local output="$2"
  if echo "$output" | grep -qiE "revert|error|execution reverted"; then
    echo "  PASS — $label"
    ((pass++))
  else
    echo "  FAIL — $label (ожидался revert, получено: $output)"
    ((fail++))
  fi
}

echo ""
echo "═══ DIUReputation — access control ═══"

echo "→ addXP без авторизации должен revert"
OUT=$(cast send "$REPUTATION" \
  "addXP(address,uint64)" "$DUMMY_ADDR" 100 \
  --private-key "$ATTACKER_KEY" \
  --rpc-url "$RPC" 2>&1 || true)
check "addXP unauthorized" "$OUT"

# TODO: добавить recordLogin, grantBadge после проверки сигнатур

echo ""
echo "═══ DIUToken — access control ═══"

echo "→ mint без авторизации должен revert"
OUT=$(cast send "$TOKEN" \
  "mint(address,uint256)" "$DUMMY_ADDR" 1000 \
  --private-key "$ATTACKER_KEY" \
  --rpc-url "$RPC" 2>&1 || true)
check "mint unauthorized" "$OUT"

# TODO: добавить pause(), unpause() checks

echo ""
echo "═══ DIUProgress — access control ═══"

echo "→ recordSimulation без авторизации должен revert"
OUT=$(cast send "$PROGRESS" \
  "recordSimulation(address,uint256,uint256)" "$DUMMY_ADDR" 0 50 \
  --private-key "$ATTACKER_KEY" \
  --rpc-url "$RPC" 2>&1 || true)
check "recordSimulation unauthorized" "$OUT"

# TODO: добавить getExportSnapshot check

echo ""
echo "═══ DIURegistry — access control ═══"

echo "→ verify() без owner прав должен revert"
OUT=$(cast send "$REGISTRY" \
  "verify(address)" "$DUMMY_ADDR" \
  --private-key "$ATTACKER_KEY" \
  --rpc-url "$RPC" 2>&1 || true)
check "verify unauthorized" "$OUT"

echo ""
echo "═══ DIUAchievements — access control ═══"

echo "→ mint badge без авторизации должен revert"
OUT=$(cast send "$ACHIEVEMENTS" \
  "mint(address,uint64)" "$DUMMY_ADDR" 1 \
  --private-key "$ATTACKER_KEY" \
  --rpc-url "$RPC" 2>&1 || true)
check "achievements mint unauthorized" "$OUT"

echo ""
echo "═══ Результаты ═══"
echo "PASS: $pass | FAIL: $fail"
if [[ $fail -gt 0 ]]; then
  echo "КРИТИЧНО: $fail проверок провалены — возможные дыры в access control!"
  exit 1
fi
echo "Все проверки пройдены."
