'use client'

import Link from 'next/link'
import type { PoolState } from 'deeppoolsdk'
import { LAMPORTS_PER_SOL } from 'deeppoolsdk'

export const PoolRow = ({ pool }: { pool: PoolState }) => {
  const solDepth = pool.solReserve / LAMPORTS_PER_SOL
  const tokenDepth = pool.tokenReserve / 1e6
  return (
    <Link
      href={`/pool/${pool.tokenMint}`}
      className="pool-row"
      style={{
        display: 'flex',
        justifyContent: 'space-between',
        alignItems: 'center',
        padding: '14px 16px',
        borderBottom: '1px solid var(--border-color)',
        textDecoration: 'none',
        color: 'var(--foreground)',
        gap: '12px',
      }}
      onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--surface-hover)')}
      onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
    >
      <div style={{ minWidth: 0 }}>
        <div className="font-mono" style={{ fontSize: '13px', fontWeight: 600 }}>
          {pool.tokenMint.slice(0, 4)}...{pool.tokenMint.slice(-4)}
        </div>
      </div>
      <div style={{ textAlign: 'right', flexShrink: 0 }}>
        <div className="font-mono" style={{ fontSize: '13px', fontWeight: 500 }}>
          {solDepth.toFixed(4)} SOL
        </div>
        <div
          className="font-mono"
          style={{ fontSize: '11px', color: 'var(--muted)', marginTop: '2px' }}
        >
          {tokenDepth.toFixed(2)} tokens
        </div>
      </div>
    </Link>
  )
}
