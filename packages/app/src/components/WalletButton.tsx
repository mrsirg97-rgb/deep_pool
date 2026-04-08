'use client'

import { useState, useRef, useEffect } from 'react'
import { useWallet } from '@solana/wallet-adapter-react'
import { useWalletModal } from '@solana/wallet-adapter-react-ui'
import { useRouter } from 'next/navigation'

export function WalletButton() {
  const { publicKey, disconnect, connected } = useWallet()
  const { setVisible } = useWalletModal()
  const router = useRouter()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  // Close dropdown on outside click
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [])

  if (!connected || !publicKey) {
    return (
      <button
        onClick={() => setVisible(true)}
        style={{
          height: '32px', padding: '0 12px', border: 'none',
          borderRadius: '0 0.5rem 0.5rem 0',
          background: 'var(--surface)', color: 'var(--foreground)',
          fontSize: '12px', fontWeight: 500, cursor: 'pointer',
        }}
        onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--surface-hover)')}
        onMouseLeave={(e) => (e.currentTarget.style.background = 'var(--surface)')}
      >
        Connect
      </button>
    )
  }

  const addr = publicKey.toBase58()
  const short = `${addr.slice(0, 4)}..${addr.slice(-4)}`

  return (
    <div ref={ref} style={{ position: 'relative' }}>
      <button
        onClick={() => setOpen(!open)}
        style={{
          height: '32px', padding: '0 12px', border: 'none',
          borderRadius: '0 0.5rem 0.5rem 0',
          background: open ? 'var(--surface-hover)' : 'var(--surface)',
          color: 'var(--foreground)',
          fontSize: '12px', fontWeight: 500, fontFamily: "'Space Mono', monospace",
          cursor: 'pointer',
        }}
        onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--surface-hover)')}
        onMouseLeave={(e) => { if (!open) e.currentTarget.style.background = 'var(--surface)' }}
      >
        {short}
      </button>

      {open && (
        <div style={{
          position: 'absolute', top: '36px', right: 0, zIndex: 50,
          background: 'rgba(30, 30, 30, 0.95)',
          border: '1px solid rgba(255, 255, 255, 0.1)',
          borderRadius: '8px', padding: '4px',
          backdropFilter: 'blur(20px)',
          minWidth: '160px',
        }}>
          <button
            onClick={() => { navigator.clipboard.writeText(addr); setOpen(false) }}
            style={{
              display: 'block', width: '100%', textAlign: 'left',
              padding: '8px 12px', border: 'none', borderRadius: '6px',
              background: 'transparent', color: 'var(--foreground)',
              fontSize: '13px', cursor: 'pointer',
            }}
            onMouseEnter={(e) => (e.currentTarget.style.background = 'rgba(255,255,255,0.08)')}
            onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
          >
            Copy Address
          </button>
          <button
            onClick={() => { router.push('/portfolio'); setOpen(false) }}
            style={{
              display: 'block', width: '100%', textAlign: 'left',
              padding: '8px 12px', border: 'none', borderRadius: '6px',
              background: 'transparent', color: 'var(--foreground)',
              fontSize: '13px', cursor: 'pointer',
            }}
            onMouseEnter={(e) => (e.currentTarget.style.background = 'rgba(255,255,255,0.08)')}
            onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
          >
            Portfolio
          </button>
          <div style={{ height: '1px', background: 'rgba(255,255,255,0.08)', margin: '4px 0' }} />
          <button
            onClick={() => { disconnect(); setOpen(false) }}
            style={{
              display: 'block', width: '100%', textAlign: 'left',
              padding: '8px 12px', border: 'none', borderRadius: '6px',
              background: 'transparent', color: 'var(--danger)',
              fontSize: '13px', cursor: 'pointer',
            }}
            onMouseEnter={(e) => (e.currentTarget.style.background = 'rgba(255,255,255,0.08)')}
            onMouseLeave={(e) => (e.currentTarget.style.background = 'transparent')}
          >
            Disconnect
          </button>
        </div>
      )}
    </div>
  )
}
