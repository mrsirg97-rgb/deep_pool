# DeepPool

ProgramID: CcwF61GW14AcxCS4E2zedHXdFXy8x8GQPvfxZrs2x2eT

- read the [design](./docs/design.md).
- 16/16 passing kani proofs in [verification](./docs/verification.md).
- 19 proptest properties × 10,000 cases in [properties](./docs/properties.md).
- internal [audit](./docs/audit.md).
- develop on deep_pool and use the test suite with the [sdk](./packages/sdk/readme.md).

## kani + proptest

```bash
anchor build
cargo kani
cargo test -p deep_pool --test math_proptests
```

## sim

```bash
python3 sim/deeppool_sim.py
```

Brightside Solutions, 2026
