# DIUProgress v2 — Deployment Log (ADR D-087)

- **Network**: Arbitrum Sepolia (chainId 421614)
- **Deploy date**: 14 June 2026
- **Deployer**: `0x5249D73BeecF9aBf64b38AbF176633056Dd4fD7C` (key `~/.keys/diu-deployer`)
- **Source**: diu monorepo `contracts/src/progress.rs` (`--features progress`)
- **Status**: deployed + initialized + smoke-tested. Ownership transfer to Gnosis Safe pending (Phase 3).

## Addresses

| Item | Value |
|------|-------|
| DIUProgress v2 | `0x553dfc81b24920ce374a6eb0847187cbdd3c82ea` |
| Owner (current) | `0x5249D73BeecF9aBf64b38AbF176633056Dd4fD7C` (deployer) |
| reputation_contract (init) | `0x8740f9d110133ff5efa0fb562e62ab92a466cdc5` |
| max_experiment_id / max_module_id | 50 / 50 |
| storage_version | 2 |
| DIUProgress v1 (historical, expired) | `0xb1c4edc73aae322f62cda57f84f303761ca3e347` |

Arbiscan: https://sepolia.arbiscan.io/address/0x553dfc81b24920ce374a6eb0847187cbdd3c82ea

## Transactions

| Step | Tx hash |
|------|---------|
| Deploy | `0xd8d8a0f3a2639c8f3763d6e088fbffec900e802521584a0eb8d0adccefe2116e` |
| Activation | `0x83c73799cfb0f4fe0dfcf4fc973f8a729a5061a2858346805a500e43cbb7dae1` |
| initialize | `0xc9e936b3579b912caed809a6fbd77c4224ba80f9aa3f3002ffe3b31b793d522d` |
| smoke attest_result | `0xaa4917e9c5d6024953e19a20392276743e19726151cb03a579490540a1e85c1f` |
| smoke attest_with_sig (real EIP-712 sig) | `0x816ce81968ab04fbaebdbe614a8bbc543072237ab9d080d4bcf81b5eca037d6f` |

## Smoke results

- `attest_result(0xaaaa…)` → has_attested=true, count=1, event `via_relay=false`.
- `attest_with_sig(0xbbbb…, deployer, deadline, sig)` → recovered==attester, count=1, relay_nonce 0→1, event `via_relay=true`. Signature: EIP-712 over domain {name:"DIUProgress", version:"2", chainId:421614, verifyingContract: v2 addr}, signed via `cast wallet sign --data` with the deployer key — validates the hand-rolled EIP-712 digest + ecrecover precompile + v-normalization.

## Build / size note (reproducibility)

- Plain `cargo stylus deploy --features progress` → 25.1 KB compressed, exceeds the 24 KB on-chain limit.
- Fit achieved with binaryen **wasm-opt v130**: `-Oz --converge --strip-debug --strip-producers --enable-bulk-memory --enable-sign-ext` over the cdylib artifact (`target/wasm32-unknown-unknown/release/deps/diu_contracts.wasm`, 130 KB → 73 KB uncompressed → 24.5 KB compressed), plus root `Cargo.toml` `strip=true` + `[profile.release.package.diu-contracts] opt-level="z"`.
- cargo-stylus 0.10.0 does not auto-run wasm-opt and `deploy` lacks `--wasm-file`; the artifact was optimized in-place before deploy. Not reproducible by a plain `cargo stylus deploy` — revisit before mainnet.

## Notes

- First deploy ran without `--features` → default entrypoint DIURegistry; an uninitialized DIURegistry landed at `0x1d3f2a9b051cebf3f054ed278d88d7c81f147262` (harmless, ignore).
- Pending (Phase 3+): transfer_ownership → DIUMultiSig `0xec142b5854b8a147e256f23e62d87f163c37ffd9` + accept_ownership; address-book update; migration 009 + backend/frontend (D-087 phases 3–5).
