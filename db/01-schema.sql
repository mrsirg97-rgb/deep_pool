CREATE TABLE IF NOT EXISTS pools (
    pool_id            SERIAL PRIMARY KEY,
    pubkey             TEXT NOT NULL UNIQUE,
    config             TEXT NOT NULL,
    token_mint         TEXT NOT NULL,
    lp_mint            TEXT NOT NULL,
    creator            TEXT NOT NULL,
    sol_initial        BIGINT NOT NULL,
    tokens_initial     BIGINT NOT NULL,
    lp_supply_initial  BIGINT NOT NULL,
    slot               BIGINT NOT NULL,
    signature          TEXT NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS pools_token_mint_idx ON pools(token_mint);
CREATE INDEX IF NOT EXISTS pools_creator_idx    ON pools(creator);

CREATE TABLE IF NOT EXISTS reserves (
    reserve_id     SERIAL PRIMARY KEY,
    pool_id        INT NOT NULL REFERENCES pools(pool_id),
    sol_reserve    BIGINT NOT NULL,
    token_reserve  BIGINT NOT NULL,
    lp_supply      BIGINT NOT NULL,
    last_slot      BIGINT NOT NULL,
    signature      TEXT NOT NULL,
    inner_ix_idx   INT NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL,
    UNIQUE (signature, inner_ix_idx)
);

CREATE INDEX IF NOT EXISTS reserves_pool_slot_idx ON reserves(pool_id, last_slot DESC, reserve_id DESC);

CREATE TABLE IF NOT EXISTS swaps (
    swap_id              SERIAL PRIMARY KEY,
    pool_id              INT NOT NULL REFERENCES pools(pool_id),
    user_pk              TEXT NOT NULL,
    sol_source           TEXT NOT NULL,
    is_buy               BOOLEAN NOT NULL,
    amount_in_gross      BIGINT NOT NULL,
    amount_in_net        BIGINT NOT NULL,
    amount_out_gross     BIGINT NOT NULL,
    amount_out_net       BIGINT NOT NULL,
    fee                  BIGINT NOT NULL,
    sol_reserve_after    BIGINT NOT NULL,
    token_reserve_after  BIGINT NOT NULL,
    total_swaps          BIGINT NOT NULL,
    slot                 BIGINT NOT NULL,
    signature            TEXT NOT NULL,
    inner_ix_idx         INT NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL,
    UNIQUE (signature, inner_ix_idx)
);

CREATE INDEX IF NOT EXISTS swaps_pool_created_idx ON swaps(pool_id, created_at DESC);
CREATE INDEX IF NOT EXISTS swaps_user_created_idx ON swaps(user_pk, created_at DESC);

CREATE TABLE IF NOT EXISTS liquidity_events (
    liquidity_id         SERIAL PRIMARY KEY,
    pool_id              INT NOT NULL REFERENCES pools(pool_id),
    provider             TEXT NOT NULL,
    is_add               BOOLEAN NOT NULL,
    sol_amount_gross     BIGINT NOT NULL,
    sol_amount_net       BIGINT NOT NULL,
    tokens_amount_gross  BIGINT NOT NULL,
    tokens_amount_net    BIGINT NOT NULL,
    lp_user_amount       BIGINT NOT NULL,  -- lp_to_provider on add, lp_burned on remove
    lp_locked            BIGINT NOT NULL,  -- 0 on remove
    lp_supply_after      BIGINT NOT NULL,
    slot                 BIGINT NOT NULL,
    signature            TEXT NOT NULL,
    inner_ix_idx         INT NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL,
    UNIQUE (signature, inner_ix_idx)
);

CREATE INDEX IF NOT EXISTS liquidity_pool_created_idx     ON liquidity_events(pool_id, created_at DESC);
CREATE INDEX IF NOT EXISTS liquidity_provider_created_idx ON liquidity_events(provider, created_at DESC);

CREATE TABLE IF NOT EXISTS indexer_state (
    id INT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    last_processed_slot BIGINT NOT NULL
);
