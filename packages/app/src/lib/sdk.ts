import type { ReadOptions } from 'deeppoolsdk'

// Read NEXT_PUBLIC_INDEXER_URL from build-time env (Next.js inlines NEXT_PUBLIC_*
// at compile time). When set — e.g. via the docker compose stack which bakes
// http://localhost:8080 into the build — pool reads route through the indexer
// first and silently fall back to RPC on any failure. When unset, every read
// goes straight to RPC.
const INDEXER_URL = process.env.NEXT_PUBLIC_INDEXER_URL

export const readOptions = (): ReadOptions | undefined =>
  INDEXER_URL ? { indexer: INDEXER_URL } : undefined

export const indexerUrl = (): string | undefined => INDEXER_URL || undefined
