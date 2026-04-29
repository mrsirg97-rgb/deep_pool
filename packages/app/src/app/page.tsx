'use client'

import { useCallback, useEffect, useState } from 'react'
import { useConnection } from '@solana/wallet-adapter-react'
import { PublicKey } from '@solana/web3.js'
import { getPoolByAddress, POOL_ACCOUNT_SIZE, PROGRAM_ID, getVaultPda } from 'deeppoolsdk'
import type { PoolState } from 'deeppoolsdk'
import { Header } from '@/components/Header'
import { PoolRow } from '@/components/PoolRow'
import { readOptions } from '@/lib/sdk'
import { useIndexerEvent } from '@/lib/bus'

export default function Home() {
  const { connection } = useConnection()
  const [pools, setPools] = useState<PoolState[]>([])
  const [loading, setLoading] = useState(true)
  const [search, setSearch] = useState('')
  useEffect(() => {
    const fetchPools = async () => {
      try {
        // Find all pool accounts owned by the program
        const accounts = await connection.getProgramAccounts(PROGRAM_ID, {
          filters: [{ dataSize: POOL_ACCOUNT_SIZE }],
        })

        const poolStates: PoolState[] = []
        for (const { pubkey } of accounts) {
          const pool = await getPoolByAddress(connection, pubkey, readOptions())
          if (pool) poolStates.push(pool)
        }

        // Sort by SOL depth descending
        poolStates.sort((a, b) => b.solReserve - a.solReserve)
        setPools(poolStates)
      } catch (e) {
        console.error('Failed to fetch pools:', e)
      } finally {
        setLoading(false)
      }
    }
    fetchPools()
  }, [connection])

  // Live append: new pools created via PoolCreated events show up without
  // refresh. Reserves updates would need a pool_id ↔ address map (events
  // carry pool_id; PoolState has address) — wire that in v2.
  useIndexerEvent(
    'pool',
    useCallback((frame) => {
      setPools((prev) => {
        if (prev.some((p) => p.address === frame.pubkey)) return prev
        const [vault] = getVaultPda(new PublicKey(frame.pubkey))
        const next: PoolState = {
          address: frame.pubkey,
          config: frame.config,
          tokenMint: frame.token_mint,
          tokenVault: vault.toBase58(),
          lpMint: frame.lp_mint,
          initialSol: frame.sol_initial,
          initialTokens: frame.tokens_initial,
          // Reserves arrive in a separate frame within the same block; we
          // bootstrap with initial values and let v2's reserves wiring
          // catch up the live numbers.
          solReserve: frame.sol_initial,
          tokenReserve: frame.tokens_initial,
          price:
            frame.tokens_initial > 0 ? frame.sol_initial / 1e9 / (frame.tokens_initial / 1e6) : 0,
        }
        return [...prev, next].sort((a, b) => b.solReserve - a.solReserve)
      })
    }, []),
  )

  return (
    <>
      <Header />
      <main style={{ maxWidth: '900px', margin: '0 auto', padding: '24px' }}>
        <div
          style={{
            display: 'flex',
            justifyContent: 'space-between',
            alignItems: 'center',
            marginBottom: '24px',
            gap: '16px',
          }}
        >
          <h1 style={{ fontSize: '24px', fontWeight: 700 }}>Pools</h1>
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search token..."
            className="font-mono"
            style={{
              padding: '8px 12px',
              borderRadius: '8px',
              border: '1px solid var(--border-color)',
              background: 'var(--surface)',
              color: 'var(--foreground)',
              fontSize: '13px',
              outline: 'none',
              width: '200px',
            }}
          />
        </div>

        <div
          style={{
            borderRadius: '12px',
            border: '1px solid var(--border-color)',
            overflow: 'hidden',
          }}
        >
          <div
            style={{
              display: 'flex',
              justifyContent: 'space-between',
              padding: '10px 16px',
              background: 'var(--surface)',
              fontSize: '11px',
              fontWeight: 600,
              color: 'var(--muted)',
              textTransform: 'uppercase',
              letterSpacing: '0.5px',
            }}
          >
            <span>Token</span>
            <span>Depth</span>
          </div>

          {loading ? (
            <div
              style={{
                padding: '40px',
                textAlign: 'center',
                color: 'var(--muted)',
                fontSize: '14px',
              }}
            >
              Loading pools...
            </div>
          ) : pools.length === 0 ? (
            <div
              style={{
                padding: '40px',
                textAlign: 'center',
                color: 'var(--muted)',
                fontSize: '14px',
              }}
            >
              No pools found. Create one to get started.
            </div>
          ) : (
            pools
              .filter((p) => !search || p.tokenMint.toLowerCase().includes(search.toLowerCase()))
              .map((pool) => <PoolRow key={pool.address} pool={pool} />)
          )}
        </div>
      </main>
    </>
  )
}
