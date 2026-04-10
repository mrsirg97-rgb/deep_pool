'use client'

import { useState, useEffect, use } from 'react'
import { useConnection } from '@solana/wallet-adapter-react'
import { LAMPORTS_PER_SOL } from '@solana/web3.js'
import { getPoolsForMint } from 'deeppoolsdk'
import type { PoolState } from 'deeppoolsdk'
import { Header } from '@/components/Header'
import { SwapPanel } from '@/components/SwapPanel'
import { LPPanel } from '@/components/LPPanel'

export default function PoolPage({ params }: { params: Promise<{ mint: string }> }) {
  const { mint } = use(params)
  const { connection } = useConnection()
  const [pool, setPool] = useState<PoolState | null>(null)
  const [loading, setLoading] = useState(true)
  const [tab, setTab] = useState<'swap' | 'lp'>('swap')

  const refresh = async () => {
    try {
      const pools = await getPoolsForMint(connection, mint)
      setPool(pools.length > 0 ? pools[0] : null) // deepest pool
    } catch (e) {
      console.error('Failed to fetch pool:', e)
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    refresh()
    const interval = setInterval(refresh, 10000)
    return () => clearInterval(interval)
  }, [connection, mint])

  if (loading) {
    return (
      <>
        <Header />
        <main style={{ maxWidth: '600px', margin: '0 auto', padding: '24px', textAlign: 'center', color: 'var(--muted)' }}>
          Loading pool...
        </main>
      </>
    )
  }

  if (!pool) {
    return (
      <>
        <Header />
        <main style={{ maxWidth: '600px', margin: '0 auto', padding: '24px', textAlign: 'center', color: 'var(--muted)' }}>
          Pool not found for {mint.slice(0, 8)}...
        </main>
      </>
    )
  }

  const solDepth = pool.solReserve / LAMPORTS_PER_SOL
  const tokenDepth = pool.tokenReserve / 1e6

  return (
    <>
      <Header />
      <main style={{ maxWidth: '600px', margin: '0 auto', padding: '24px' }}>
        <div style={{ marginBottom: '24px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', marginBottom: '16px' }}>
            <h1 className="font-mono" style={{ fontSize: '13px', fontWeight: 600, wordBreak: 'break-all' }}>
              {mint}
            </h1>
            <button
              onClick={() => { navigator.clipboard.writeText(mint) }}
              title="Copy address"
              style={{
                background: 'none', border: 'none', cursor: 'pointer', padding: '4px',
                color: 'var(--muted)', fontSize: '14px', flexShrink: 0,
              }}
              onMouseEnter={(e) => (e.currentTarget.style.color = 'var(--foreground)')}
              onMouseLeave={(e) => (e.currentTarget.style.color = 'var(--muted)')}
            >
              &#x2398;
            </button>
          </div>

          <div style={{
            display: 'grid',
            gridTemplateColumns: '1fr 1fr',
            gap: '12px',
            marginBottom: '16px',
          }}>
            <div style={{ padding: '16px', borderRadius: '10px', background: 'var(--surface)' }}>
              <div style={{ fontSize: '11px', color: 'var(--muted)', marginBottom: '4px' }}>SOL Depth</div>
              <div className="font-mono" style={{ fontSize: '18px', fontWeight: 600 }}>
                {solDepth.toFixed(4)}
              </div>
            </div>
            <div style={{ padding: '16px', borderRadius: '10px', background: 'var(--surface)' }}>
              <div style={{ fontSize: '11px', color: 'var(--muted)', marginBottom: '4px' }}>Token Reserve</div>
              <div className="font-mono" style={{ fontSize: '18px', fontWeight: 600 }}>
                {tokenDepth.toFixed(2)}
              </div>
            </div>
            <div style={{ padding: '16px', borderRadius: '10px', background: 'var(--surface)' }}>
              <div style={{ fontSize: '11px', color: 'var(--muted)', marginBottom: '4px' }}>Price (SOL/token)</div>
              <div className="font-mono" style={{ fontSize: '18px', fontWeight: 600 }}>
                {pool.price.toFixed(9)}
              </div>
            </div>
            <div style={{ padding: '16px', borderRadius: '10px', background: 'var(--surface)' }}>
              <div style={{ fontSize: '11px', color: 'var(--muted)', marginBottom: '4px' }}>Total Swaps</div>
              <div className="font-mono" style={{ fontSize: '18px', fontWeight: 600 }}>
                {pool.totalSwaps}
              </div>
            </div>
          </div>
        </div>

        <div style={{
          borderRadius: '12px',
          border: '1px solid var(--border-color)',
          overflow: 'hidden',
        }}>
          <div style={{ display: 'flex', borderBottom: '1px solid var(--border-color)' }}>
            <button
              onClick={() => setTab('swap')}
              style={{
                flex: 1, padding: '12px', border: 'none', fontSize: '14px', fontWeight: 600,
                cursor: 'pointer',
                background: tab === 'swap' ? 'var(--surface-hover)' : 'transparent',
                color: tab === 'swap' ? 'var(--foreground)' : 'var(--muted)',
                borderBottom: tab === 'swap' ? '2px solid var(--accent)' : '2px solid transparent',
              }}
            >
              Swap
            </button>
            <button
              onClick={() => setTab('lp')}
              style={{
                flex: 1, padding: '12px', border: 'none', fontSize: '14px', fontWeight: 600,
                cursor: 'pointer',
                background: tab === 'lp' ? 'var(--surface-hover)' : 'transparent',
                color: tab === 'lp' ? 'var(--foreground)' : 'var(--muted)',
                borderBottom: tab === 'lp' ? '2px solid var(--accent)' : '2px solid transparent',
              }}
            >
              Liquidity
            </button>
          </div>

          {tab === 'swap' ? (
            <SwapPanel mint={mint} pool={pool} />
          ) : (
            <LPPanel mint={mint} pool={pool} />
          )}
        </div>
      </main>
    </>
  )
}
