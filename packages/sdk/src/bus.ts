// Live event bus over the indexer's WebSocket.
//
// Single global connection per indexer URL. Components subscribe to typed
// "rooms" (event kinds) and the bus dispatches each incoming frame to the
// matching subscribers. On reconnect, gap-fill via REST so subscribers see
// every event that landed during the disconnect window — there's no
// "missed-event" gap from the consumer's perspective.
//
// Wire format mirrors the indexer's BroadcastFrame: `#[serde(tag = "kind",
// rename_all = "snake_case")]` flattens the row's fields into the frame, so
// each WS message is a discriminated union keyed by `frame.kind`.

import type { IndexerPoolRow, IndexerReservesRow, LiquidityRow, SwapRow } from './types'
import { getLiquidityHistory, getSwapHistory } from './indexer'

// ============================================================================
// Frame types — discriminated union matching the indexer's serde output
// ============================================================================

export type BroadcastFrame =
  | ({ kind: 'pool' } & IndexerPoolRow)
  | ({ kind: 'swap' } & SwapRow)
  | ({ kind: 'liquidity' } & LiquidityRow)
  | ({ kind: 'reserves' } & IndexerReservesRow)

export type BroadcastKind = BroadcastFrame['kind']

type FrameOf<K extends BroadcastKind> = Extract<BroadcastFrame, { kind: K }>

export type FrameHandler<K extends BroadcastKind> = (frame: FrameOf<K>) => void

export type ConnectionState = 'connected' | 'disconnected'

// ============================================================================
// Bus interface
// ============================================================================

export interface IndexerBus {
  on<K extends BroadcastKind>(kind: K, handler: FrameHandler<K>): () => void
  onConnectionChange(handler: (state: ConnectionState) => void): () => void
  close(): void
}

// ============================================================================
// Implementation
// ============================================================================

const RECONNECT_DELAY_MS = 1_000
const GAP_FILL_LIMIT = 500

interface BusInternal {
  ws: WebSocket | null
  closed: boolean
  hasEverConnected: boolean
  lastSeenAt: string | null // ISO timestamp; null until first frame
  seenPoolIds: Set<number>
  handlers: {
    pool: Set<FrameHandler<'pool'>>
    swap: Set<FrameHandler<'swap'>>
    liquidity: Set<FrameHandler<'liquidity'>>
    reserves: Set<FrameHandler<'reserves'>>
  }
  connectionHandlers: Set<(state: ConnectionState) => void>
}

export function createIndexerBus(indexer: string): IndexerBus {
  // No-op bus when running outside a browser (SSR, tests). WebSocket is
  // global in browsers; if it's undefined here we simply never connect.
  if (typeof WebSocket === 'undefined') {
    return {
      on: () => () => {},
      onConnectionChange: () => () => {},
      close: () => {},
    }
  }

  const state: BusInternal = {
    ws: null,
    closed: false,
    hasEverConnected: false,
    lastSeenAt: null,
    seenPoolIds: new Set(),
    handlers: {
      pool: new Set(),
      swap: new Set(),
      liquidity: new Set(),
      reserves: new Set(),
    },
    connectionHandlers: new Set(),
  }

  const wsUrl = indexer.replace(/^http/, 'ws') + '/events'

  function notifyConnection(s: ConnectionState) {
    for (const h of state.connectionHandlers) {
      try {
        h(s)
      } catch {}
    }
  }

  function handleFrame(frame: BroadcastFrame) {
    // Track for gap-fill: max created_at across all frames + observed pool_ids.
    if (frame.created_at && (!state.lastSeenAt || frame.created_at > state.lastSeenAt)) {
      state.lastSeenAt = frame.created_at
    }
    if (frame.kind === 'pool') state.seenPoolIds.add(frame.pool_id)

    const handlers = state.handlers[frame.kind] as Set<FrameHandler<typeof frame.kind>>
    for (const h of handlers) {
      try {
        ;(h as FrameHandler<typeof frame.kind>)(frame as FrameOf<typeof frame.kind>)
      } catch {
        // Handler errors must not break the dispatch loop.
      }
    }
  }

  async function gapFill() {
    // No reference point yet — first connect, nothing to fill.
    if (!state.lastSeenAt) return

    const since = new Date(state.lastSeenAt)

    // Swaps + liquidity have proper `since` filters. Order matters: emit
    // chronologically so subscribers reduce in the order events happened.
    try {
      const swaps = await getSwapHistory({ indexer, since, limit: GAP_FILL_LIMIT })
      swaps
        .sort((a, b) => a.created_at.localeCompare(b.created_at))
        .forEach((s) => handleFrame({ kind: 'swap', ...s }))
    } catch {
      // Surface as a connection-state blip rather than aborting the bus.
    }

    try {
      const liq = await getLiquidityHistory({
        indexer,
        since,
        limit: GAP_FILL_LIMIT,
      })
      liq
        .sort((a, b) => a.created_at.localeCompare(b.created_at))
        .forEach((l) => handleFrame({ kind: 'liquidity', ...l }))
    } catch {}

    // Pools: list endpoint has no `since` filter, but the universe is small.
    // Refetch all and emit any pool_ids we haven't seen before.
    try {
      const resp = await fetch(`${indexer}/api/pools`)
      if (resp.ok) {
        const pools: IndexerPoolRow[] = await resp.json()
        pools
          .filter((p) => !state.seenPoolIds.has(p.pool_id))
          .sort((a, b) => a.created_at.localeCompare(b.created_at))
          .forEach((p) => handleFrame({ kind: 'pool', ...p }))
      }
    } catch {}

    // Reserves are NOT gap-filled directly — they have no since-filtered
    // list endpoint. Subscribers that need reserves freshness should also
    // listen to `swap` and `liquidity` (their post-state reserves are on
    // the event payload as `*_reserve_after`).
  }

  function connect() {
    if (state.closed) return
    state.ws = new WebSocket(wsUrl)

    state.ws.addEventListener('open', () => {
      notifyConnection('connected')
      if (state.hasEverConnected) {
        // Reconnect after a disconnect — fill the gap.
        gapFill()
      }
      state.hasEverConnected = true
    })

    state.ws.addEventListener('message', (ev) => {
      try {
        const frame = JSON.parse(typeof ev.data === 'string' ? ev.data : '') as BroadcastFrame
        if (frame && typeof frame.kind === 'string') handleFrame(frame)
      } catch {
        // Drop malformed frames silently.
      }
    })

    state.ws.addEventListener('close', () => {
      state.ws = null
      notifyConnection('disconnected')
      if (!state.closed) {
        setTimeout(connect, RECONNECT_DELAY_MS)
      }
    })

    state.ws.addEventListener('error', () => {
      // The browser also fires `close` after an error, so reconnect is
      // handled there. Nothing to do here.
    })
  }

  connect()

  return {
    on(kind, handler) {
      // The handler-set types are kind-specific, but the public `on` is
      // generic — we know the cast is safe because we index by `kind`.
      const set = state.handlers[kind] as unknown as Set<typeof handler>
      set.add(handler)
      return () => {
        set.delete(handler)
      }
    },
    onConnectionChange(handler) {
      state.connectionHandlers.add(handler)
      return () => {
        state.connectionHandlers.delete(handler)
      }
    },
    close() {
      state.closed = true
      state.ws?.close()
      state.handlers.pool.clear()
      state.handlers.swap.clear()
      state.handlers.liquidity.clear()
      state.handlers.reserves.clear()
      state.connectionHandlers.clear()
    },
  }
}
