#!/bin/bash
# Runs after 01-schema.sql (Postgres init scripts execute in lexical order).
# Creates the indexer's least-privilege role and grants exactly the rights
# the indexer needs — nothing more.
#
# Runs as $POSTGRES_USER (superuser) against $POSTGRES_DB.
# INDEXER_DB_PASSWORD is supplied via docker-compose.yml env.

set -euo pipefail

if [[ -z "${INDEXER_DB_PASSWORD:-}" ]]; then
    echo "ERROR: INDEXER_DB_PASSWORD env var not set. Cannot create indexer role." >&2
    exit 1
fi

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<-EOSQL
    -- Least-privilege role. Only LOGIN + what's granted below; no CREATE,
    -- no DELETE, no TRUNCATE, no access to schemas beyond public.
    CREATE ROLE deep_pool_indexer LOGIN PASSWORD '${INDEXER_DB_PASSWORD}';

    -- Connect + schema usage
    GRANT CONNECT ON DATABASE ${POSTGRES_DB} TO deep_pool_indexer;
    GRANT USAGE ON SCHEMA public TO deep_pool_indexer;

    -- Event tables. INSERT for the writer; SELECT for the API; UPDATE
    -- granted defensively in case future idempotency patterns need it.
    -- No DELETE, no TRUNCATE.
    GRANT SELECT, INSERT, UPDATE ON pools             TO deep_pool_indexer;
    GRANT SELECT, INSERT, UPDATE ON reserves          TO deep_pool_indexer;
    GRANT SELECT, INSERT, UPDATE ON swaps             TO deep_pool_indexer;
    GRANT SELECT, INSERT, UPDATE ON liquidity_events  TO deep_pool_indexer;

    -- Checkpoint table. The writer uses INSERT ... ON CONFLICT DO UPDATE
    -- so it needs both privileges; the API never writes here.
    GRANT SELECT, INSERT, UPDATE ON indexer_state TO deep_pool_indexer;

    -- SERIAL columns generate sequences (pools_pool_id_seq, etc.). INSERT
    -- on a table doesn't implicitly grant nextval() — must grant USAGE on
    -- the sequence too. ALL SEQUENCES covers the four event tables.
    GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO deep_pool_indexer;

    -- Explicitly deny schema-modifying rights. (Already implicit from not
    -- granting, but stating it makes the security model auditable.)
    REVOKE CREATE ON SCHEMA public FROM deep_pool_indexer;
EOSQL

echo "created deep_pool_indexer role with least-privilege grants"
