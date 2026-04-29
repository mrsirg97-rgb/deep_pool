'use client'

// React glue around the SDK's IndexerBus. One bus instance per provider —
// mount the provider once at the app shell so every page shares the same
// WebSocket connection and gap-fill state.
//
// Components subscribe via `useIndexerEvent(kind, handler)`. The hook
// auto-unsubscribes on unmount and re-subscribes if the handler reference
// changes — so wrap callbacks in `useCallback` to avoid resubscribing on
// every render.

import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import {
  createIndexerBus,
  type BroadcastKind,
  type ConnectionState,
  type FrameHandler,
  type IndexerBus,
} from 'deeppoolsdk'
import { indexerUrl } from './sdk'

const BusContext = createContext<IndexerBus | null>(null)

export function IndexerBusProvider({ children }: { children: ReactNode }) {
  const [bus, setBus] = useState<IndexerBus | null>(null)

  useEffect(() => {
    const url = indexerUrl()
    if (!url) {
      // No indexer configured — bus stays null, hooks are no-ops, pages
      // continue to function via RPC reads.
      return
    }
    const b = createIndexerBus(url)
    setBus(b)
    return () => {
      b.close()
    }
  }, [])

  return <BusContext.Provider value={bus}>{children}</BusContext.Provider>
}

// Subscribe a typed handler to one event kind. Wrap your handler in
// `useCallback` if you don't want it to resubscribe on every render.
export function useIndexerEvent<K extends BroadcastKind>(kind: K, handler: FrameHandler<K>): void {
  const bus = useContext(BusContext)
  // Latest-handler ref so identity changes don't churn subscriptions.
  const ref = useRef(handler)
  useEffect(() => {
    ref.current = handler
  }, [handler])

  useEffect(() => {
    if (!bus) return
    return bus.on(kind, ((frame: Parameters<FrameHandler<K>>[0]) => {
      ref.current(frame)
    }) as FrameHandler<K>)
  }, [bus, kind])
}

// Connection-state hook — useful for "live" / "reconnecting" status badges.
export function useIndexerConnection(): ConnectionState {
  const bus = useContext(BusContext)
  const [state, setState] = useState<ConnectionState>('disconnected')
  useEffect(() => {
    if (!bus) return
    return bus.onConnectionChange(setState)
  }, [bus])
  return state
}

export function useIndexerBus(): IndexerBus | null {
  return useContext(BusContext)
}

// Convenience: stable empty handler reference. Sometimes useful for
// conditionally subscribing without breaking the hook rules.
export const noopHandler = () => {}
