'use client'

import { useState, useEffect } from 'react'
import { useConnection, useWallet } from '@solana/wallet-adapter-react'
import { PublicKey, LAMPORTS_PER_SOL } from '@solana/web3.js'
import { TOKEN_2022_PROGRAM_ID } from '@solana/spl-token'
import Link from 'next/link'
import { getPoolByAddress, getLpMintPda, POOL_ACCOUNT_SIZE, PROGRAM_ID } from 'deeppoolsdk'
import type { PoolState } from 'deeppoolsdk'
import { Header } from '@/components/Header'
import { readOptions } from '@/lib/sdk'

interface LpPosition {
  pool: PoolState
  lpBalance: number
  lpSupply: number
  sharePercent: number
  solValue: number
  tokenValue: number
}

export default function PortfolioPage() {
  const { connection } = useConnection()
  const wallet = useWallet()
  const [positions, setPositions] = useState<LpPosition[]>([])
  const [loading, setLoading] = useState(false)
  const [search, setSearch] = useState('')
  useEffect(() => {
    if (!wallet.publicKey) {
      setPositions([])
      return
    }

    const fetchPositions = async () => {
      setLoading(true)
      try {
        // Get all token accounts owned by user on Token-2022
        console.log('Fetching token accounts...')
        const tokenAccounts = await connection.getParsedTokenAccountsByOwner(wallet.publicKey!, {
          programId: TOKEN_2022_PROGRAM_ID,
        })
        console.log(`Found ${tokenAccounts.value.length} token accounts`)

        // Find all pool accounts to get LP mints
        console.log('Fetching pool accounts...')
        const poolAccounts = await connection.getProgramAccounts(PROGRAM_ID, {
          filters: [{ dataSize: POOL_ACCOUNT_SIZE }],
        })
        console.log(`Found ${poolAccounts.length} pools`)

        // Build a map of LP mint → pool pubkey
        const lpMintToPool = new Map<string, PublicKey>()
        for (const { pubkey } of poolAccounts) {
          const [lpMint] = getLpMintPda(pubkey)
          lpMintToPool.set(lpMint.toBase58(), pubkey)
        }

        // Match user's token accounts against LP mints
        const lpPositions: LpPosition[] = []
        for (const { account } of tokenAccounts.value) {
          const parsed = account.data.parsed.info
          const mintAddr = parsed.mint as string
          const balance = Number(parsed.tokenAmount.amount)

          if (balance <= 0) continue

          const poolPubkey = lpMintToPool.get(mintAddr)
          if (!poolPubkey) continue

          // This is an LP token — fetch pool state
          const pool = await getPoolByAddress(connection, poolPubkey, readOptions())
          if (!pool) continue

          // Get LP supply
          const [lpMint] = getLpMintPda(poolPubkey)
          const lpMintInfo = await connection.getTokenSupply(lpMint)
          const lpSupply = Number(lpMintInfo.value.amount)

          if (lpSupply <= 0) continue

          const sharePercent = (balance / lpSupply) * 100
          const solValue = (balance / lpSupply) * pool.solReserve
          const tokenValue = (balance / lpSupply) * pool.tokenReserve

          lpPositions.push({
            pool,
            lpBalance: balance,
            lpSupply,
            sharePercent,
            solValue,
            tokenValue,
          })
        }

        // Sort by SOL value descending
        lpPositions.sort((a, b) => b.solValue - a.solValue)
        setPositions(lpPositions)
      } catch (e: any) {
        console.error('Failed to fetch LP positions:', e?.message || e)
      } finally {
        setLoading(false)
      }
    }

    fetchPositions()
  }, [connection, wallet.publicKey])

  const totalSolValue = positions.reduce((sum, p) => sum + p.solValue, 0)
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
            flexWrap: 'wrap',
          }}
        >
          <h1 style={{ fontSize: '24px', fontWeight: 700 }}>Portfolio</h1>
          <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
            {positions.length > 0 && (
              <div className="font-mono" style={{ fontSize: '14px', color: 'var(--accent)' }}>
                {(totalSolValue / LAMPORTS_PER_SOL).toFixed(4)} SOL
              </div>
            )}
            <input
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search..."
              className="font-mono"
              style={{
                padding: '8px 12px',
                borderRadius: '8px',
                border: '1px solid var(--border-color)',
                background: 'var(--surface)',
                color: 'var(--foreground)',
                fontSize: '13px',
                outline: 'none',
                width: '160px',
              }}
            />
          </div>
        </div>

        {!wallet.publicKey ? (
          <div
            style={{
              padding: '60px',
              textAlign: 'center',
              color: 'var(--muted)',
              fontSize: '14px',
              borderRadius: '12px',
              border: '1px solid var(--border-color)',
            }}
          >
            Connect wallet to view LP positions
          </div>
        ) : loading ? (
          <div
            style={{
              padding: '60px',
              textAlign: 'center',
              color: 'var(--muted)',
              fontSize: '14px',
              borderRadius: '12px',
              border: '1px solid var(--border-color)',
            }}
          >
            Loading positions...
          </div>
        ) : positions.length === 0 ? (
          <div
            style={{
              padding: '60px',
              textAlign: 'center',
              color: 'var(--muted)',
              fontSize: '14px',
              borderRadius: '12px',
              border: '1px solid var(--border-color)',
            }}
          >
            No LP positions found. Add liquidity to a pool to get started.
          </div>
        ) : (
          <div
            style={{
              borderRadius: '12px',
              border: '1px solid var(--border-color)',
              overflow: 'hidden',
            }}
          >
            {/* Header */}
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
              <span>Pool</span>
              <span>Position</span>
            </div>

            {/* Rows */}
            {positions
              .filter(
                (p) => !search || p.pool.tokenMint.toLowerCase().includes(search.toLowerCase()),
              )
              .map((pos) => (
                <Link
                  key={pos.pool.address}
                  href={`/pool/${pos.pool.tokenMint}`}
                  style={{
                    display: 'flex',
                    justifyContent: 'space-between',
                    alignItems: 'center',
                    padding: '14px 16px',
                    gap: '12px',
                    borderTop: '1px solid var(--border-color)',
                    textDecoration: 'none',
                    color: 'var(--foreground)',
                  }}
                  onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--surface-hover)')}
                  onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
                >
                  <div style={{ minWidth: 0 }}>
                    <div className="font-mono" style={{ fontSize: '13px', fontWeight: 600 }}>
                      {pos.pool.tokenMint.slice(0, 4)}...{pos.pool.tokenMint.slice(-4)}
                    </div>
                    <div style={{ fontSize: '11px', color: 'var(--muted)', marginTop: '2px' }}>
                      {(pos.lpBalance / 1e6).toFixed(4)} LP
                    </div>
                  </div>
                  <div style={{ textAlign: 'right', flexShrink: 0 }}>
                    <div className="font-mono" style={{ fontSize: '13px', fontWeight: 500 }}>
                      {(pos.solValue / LAMPORTS_PER_SOL).toFixed(4)} SOL
                    </div>
                    <div
                      className="font-mono"
                      style={{ fontSize: '11px', color: 'var(--accent)', marginTop: '2px' }}
                    >
                      {pos.sharePercent.toFixed(2)}% share
                    </div>
                  </div>
                </Link>
              ))}
          </div>
        )}
      </main>
    </>
  )
}
