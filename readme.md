# DeepPool

ProgramID: CcwF61GW14AcxCS4E2zedHXdFXy8x8GQPvfxZrs2x2eT

- read the [whitepaper](./docs/whitepaper.md).
- 14/14 passing kani proofs in [verification](./docs/verification.md).
- internal [audit](./docs/audit.md).
- develop on deep_pool and use the test suite with the [sdk](./packages/sdk/readme.md).

```bash
anchor build
cargo kani
```

## run the sim

```bash
python3 sim/deeppool_sim.py
```

Brightside Solutions, 2026
