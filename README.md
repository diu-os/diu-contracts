# DIU OS smart contracts

Arbitrum **Stylus** (Rust → WASM) contracts for [DIU OS](https://diu-os.com).

This public tree is a **showcase snapshot**, not a live mirror of the commercial workspace (`diu-platform/diu-workspace`). `progress.rs` here and in the workspace have diverged. Do not treat this repo as the operational source of truth.

**Network in use: Arbitrum Sepolia (chain id 421614).** There is no mainnet deploy.

## Deployed on Sepolia

| Contract | Address | Notes |
|----------|---------|--------|
| DIURegistry | `0x49e1b11e1037e74113a7c0ccc41e3042d4691018` | 19 Feb 2026 |
| DIUReputation | `0x8740f9d110133ff5efa0fb562e62ab92a466cdc5` | 19 Feb 2026 |
| DIUAchievements | `0x1a9783ba7966c0e7299af7ee2228e19028d8ea7e` | 19 Feb 2026 |
| DIUToken | `0xbbd9a558c049482f1be45399fec4a4c9dc1c810e` | 19 Feb 2026 |
| DIUProgress **v2** | `0x553dfc81b24920ce374a6eb0847187cbdd3c82ea` | 14 Jun 2026, ADR D-087 — **current** |
| DIUProgress v1 | `0xb1c4edc73aae322f62cda57f84f303761ca3e347` | 27 Feb 2026 — **deprecated** |
| DIUMultiSig | `0xec142b5854b8a147e256f23e62d87f163c37ffd9` | 19 May 2026. Source is **not** in this repo (commercial workspace only). |

**Current deployer:** `0x5249D73BeecF9aBf64b38AbF176633056Dd4fD7C`

**Do not use** `0x67bB4D1895D9A736F9e6076529B468ba05aeD150` — old deployer, compromised. Historical only.

`deployment_logs/final_addresses.txt` is a **superseded** February 2026 book (different addresses). Ignore it.

Log for v2: [`deployment_logs/D087_diuprogress_v2_sepolia.md`](deployment_logs/D087_diuprogress_v2_sepolia.md).

## In this tree

Five feature-gated WASM entrypoints (`registry` default, `reputation`, `achievements`, `token`, `progress`). MultiSig is not compiled here.

```bash
rustup target add wasm32-unknown-unknown
cargo test
cargo clippy -- -D warnings
cargo stylus check --endpoint https://sepolia-rollup.arbitrum.io/rpc
cargo stylus check --features progress --endpoint https://sepolia-rollup.arbitrum.io/rpc
```

Test counts in older docs (171 / 208 / 219) referred to different trees and features. Run `cargo test` in *this* checkout.

## Read-only checks (Sepolia)

```bash
export RPC="https://sepolia-rollup.arbitrum.io/rpc"
export REGISTRY="0x49e1b11e1037e74113a7c0ccc41e3042d4691018"
export PROGRESS_V2="0x553dfc81b24920ce374a6eb0847187cbdd3c82ea"

cast call $REGISTRY "owner()(address)" --rpc-url $RPC
cast call $PROGRESS_V2 "owner()(address)" --rpc-url $RPC
```

Arbiscan: [Registry](https://sepolia.arbiscan.io/address/0x49e1b11e1037e74113a7c0ccc41e3042d4691018) · [Progress v2](https://sepolia.arbiscan.io/address/0x553dfc81b24920ce374a6eb0847187cbdd3c82ea)

## Remainder (not claimed done)

Phase 4 meta-tx / `attest-relayed`, Gnosis Safe ownership handoff, event listener — backlog. DAO / staking: not started. Crowdfunding: not started.

## Related

- Platform: [diu-os.com](https://diu-os.com)
- Charter: [manifesto](https://github.com/diu-os/manifesto) (`MANIFESTO.md`)
- Showcase: [diu-os.org](https://diu-os.org)

## License

MIT. See [LICENSE](LICENSE).
