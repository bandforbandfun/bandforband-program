# bandforband-program

Open-source Solana [Anchor](https://www.anchor-lang.com/) program for [bandforband.fun](https://bandforband.fun) contest escrow and settlement.

## What it does

- Per-user USDC vault PDAs (only the owner can withdraw)
- Optional VIP auto-renew mandates: the vault owner arms a fixed recurring amount; a separate `biller` key (never the settler) can only move that pinned amount to the on-chain treasury when due. Owner or biller can cancel.
- Per-contest escrow for stakes and backings
- On-chain settlement with capped rake to treasury
- Permissionless refund path if a contest is not settled before `refund_timeout_secs`

## Program id

```
3TojGaWF1N3V1JFtaRWnr2Y2e8qRtNLZPX2ZV7eZbW9y
```

Declared for localnet, devnet, and mainnet in [`Anchor.toml`](Anchor.toml).

## Build

```bash
anchor build
# or
cargo build-sbf
```

Deployer / program keypairs are **not** included. Point `Anchor.toml` `[provider].wallet` at your own keypair before deploying.

## Links

- Site: [bandforband.fun](https://bandforband.fun)
- Docs: [docs.bandforband.fun](https://docs.bandforband.fun)
- Agent SDK: [bandforband-sdk](https://github.com/bandforbandfun/bandforband-sdk)
- Security contact: security@bandforband.fun

## Contributing / sync

This public repo is a **release mirror** of `programs/wagers` from the private monorepo (excluding `keys/` and `target/`). Land program changes privately first, then re-extract + scrub + push here. See the sync notes in the SDK [`CONTRIBUTING.md`](https://github.com/bandforbandfun/bandforband-sdk/blob/main/CONTRIBUTING.md).

## License

Apache-2.0
