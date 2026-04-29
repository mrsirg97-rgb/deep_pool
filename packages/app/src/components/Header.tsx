'use client'

import { useState, useEffect } from 'react'
import Link from 'next/link'
import { WalletButton } from './WalletButton'
import { HowItWorks } from './HowItWorks'

type NetworkId = 'devnet' | 'mainnet'

const NETWORK_OPTIONS: { id: NetworkId; label: string }[] = [
  { id: 'devnet', label: 'dev' },
  { id: 'mainnet', label: 'main' },
]

export const Header = () => {
  const [network, setNetwork] = useState<NetworkId>('devnet')
  useEffect(() => {
    const saved = localStorage.getItem('deeppool-network') as NetworkId
    if (saved) setNetwork(saved)
  }, [])

  const handleNetworkChange = (id: NetworkId) => {
    setNetwork(id)
    localStorage.setItem('deeppool-network', id)
    window.location.reload()
  }

  return (
    <header
      style={{
        display: 'flex',
        justifyContent: 'space-between',
        alignItems: 'center',
        padding: '12px 20px',
        borderBottom: '1px solid var(--border-color)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
        <Link
          href="/"
          style={{
            fontSize: '18px',
            fontWeight: 700,
            color: 'var(--accent)',
            textDecoration: 'none',
            marginRight: '8px',
          }}
        >
          DeepPool
        </Link>
        <Link
          href="/create"
          title="Create Pool"
          style={{
            fontSize: '18px',
            color: 'var(--muted)',
            textDecoration: 'none',
            width: '32px',
            height: '32px',
            borderRadius: '6px',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
          onMouseEnter={(e) => {
            e.currentTarget.style.color = 'var(--foreground)'
            e.currentTarget.style.background = 'var(--surface)'
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.color = 'var(--muted)'
            e.currentTarget.style.background = 'transparent'
          }}
        >
          +
        </Link>
        <HowItWorks />
      </div>

      {/* Control bar */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          border: '1px solid var(--border-color)',
          borderRadius: '0.5rem',
          overflow: 'visible',
          height: '32px',
        }}
      >
        <select
          value={network}
          onChange={(e) => handleNetworkChange(e.target.value as NetworkId)}
          style={{
            height: '32px',
            padding: '0 8px',
            fontSize: '11px',
            fontFamily: "'Space Mono', monospace",
            cursor: 'pointer',
            background: 'var(--surface)',
            color: 'var(--foreground)',
            border: 'none',
            borderRight: '1px solid var(--border-color)',
            borderRadius: '0.5rem 0 0 0.5rem',
            outline: 'none',
            appearance: 'none',
            WebkitAppearance: 'none',
            textAlign: 'center',
            width: '48px',
          }}
        >
          {NETWORK_OPTIONS.map((n) => (
            <option key={n.id} value={n.id}>
              {n.label}
            </option>
          ))}
        </select>

        <WalletButton />
      </div>
    </header>
  )
}
