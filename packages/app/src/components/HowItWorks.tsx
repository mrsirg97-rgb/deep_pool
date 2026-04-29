'use client'

import { useState } from 'react'

export const HowItWorks = () => {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button
        onClick={() => setOpen(true)}
        title="How it works"
        style={{
          width: '32px',
          height: '32px',
          borderRadius: '6px',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: 'transparent',
          border: 'none',
          color: 'var(--muted)',
          fontSize: '16px',
          cursor: 'pointer',
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
        ?
      </button>

      {open && (
        <div
          style={{
            position: 'fixed',
            inset: 0,
            zIndex: 100,
            background: 'rgba(0,0,0,0.7)',
            backdropFilter: 'blur(8px)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            padding: '20px',
          }}
          onClick={(e) => {
            if (e.target === e.currentTarget) setOpen(false)
          }}
        >
          <div
            style={{
              background: 'rgba(30,30,30,0.95)',
              border: '1px solid rgba(255,255,255,0.1)',
              borderRadius: '16px',
              padding: '32px',
              maxWidth: '520px',
              width: '100%',
              maxHeight: '80vh',
              overflowY: 'auto',
            }}
          >
            <div
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
                marginBottom: '24px',
              }}
            >
              <h2 style={{ fontSize: '20px', fontWeight: 700 }}>How DeepPool Works</h2>
              <button
                onClick={() => setOpen(false)}
                style={{
                  background: 'none',
                  border: 'none',
                  color: 'var(--muted)',
                  fontSize: '20px',
                  cursor: 'pointer',
                }}
              >
                &times;
              </button>
            </div>

            <div
              style={{
                display: 'flex',
                flexDirection: 'column',
                gap: '20px',
                fontSize: '14px',
                lineHeight: '1.6',
              }}
            >
              <div>
                <div style={{ fontWeight: 600, marginBottom: '4px' }}>Swap</div>
                <div style={{ color: 'var(--muted)' }}>
                  Trade any Token-2022 token for SOL and back. 0.25% fee on every swap stays in the
                  pool — no protocol extraction.
                </div>
              </div>

              <div>
                <div style={{ fontWeight: 600, marginBottom: '4px' }}>Provide Liquidity</div>
                <div style={{ color: 'var(--muted)' }}>
                  Deposit SOL + tokens proportionally. You receive LP tokens representing your
                  share. As swap fees compound, your LP appreciates.
                </div>
              </div>

              <div>
                <div style={{ fontWeight: 600, marginBottom: '4px', color: 'var(--accent)' }}>
                  LP Lock
                </div>
                <div style={{ color: 'var(--muted)' }}>
                  Pool creators lock 20% of LP permanently. Community LPs lock 7.5%. This liquidity
                  can never be withdrawn — the pool only gets deeper over time.
                </div>
              </div>

              <div>
                <div style={{ fontWeight: 600, marginBottom: '4px' }}>Self-Deepening</div>
                <div style={{ color: 'var(--muted)' }}>
                  Every swap compounds fees into reserves. Every LP deposit after creation locks
                  7.5%. The pool is a ratchet — depth only increases. More volume means tighter
                  spreads.
                </div>
              </div>

              <div>
                <div style={{ fontWeight: 600, marginBottom: '4px' }}>No Admin</div>
                <div style={{ color: 'var(--muted)' }}>
                  Pools are immutable. No fee switch, no pause, no close. Once created, the pool
                  runs forever. Pools are namespaced — each protocol gets its own isolated pool
                  space. No one can interfere with another protocol's pools. 16 Kani formal
                  verification proofs cover all math.
                </div>
              </div>

              <div
                style={{
                  padding: '12px',
                  borderRadius: '8px',
                  background: 'var(--surface)',
                  fontSize: '13px',
                }}
              >
                <div
                  style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '4px' }}
                >
                  <span style={{ color: 'var(--muted)' }}>Swap fee</span>
                  <span className="font-mono">0.25%</span>
                </div>
                <div
                  style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '4px' }}
                >
                  <span style={{ color: 'var(--muted)' }}>Protocol fee</span>
                  <span className="font-mono">0%</span>
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <span style={{ color: 'var(--muted)' }}>LP lock</span>
                  <span className="font-mono">20% creator / 7.5% LP</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      )}
    </>
  )
}
