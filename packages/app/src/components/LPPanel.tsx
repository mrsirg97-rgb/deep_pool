'use client'

import { useState, useEffect } from 'react'
import { useConnection, useWallet } from '@solana/wallet-adapter-react'
import { LAMPORTS_PER_SOL, PublicKey } from '@solana/web3.js'
import { getAssociatedTokenAddressSync } from '@solana/spl-token'
import { buildAddLiquidityTransaction, buildRemoveLiquidityTransaction, getLpMintPda, getPoolPda } from 'deeppoolsdk'
import type { PoolState } from 'deeppoolsdk'

const TOKEN_2022_PID = new PublicKey('TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb')

export function LPPanel({ mint, pool, onLpChange }: { mint: string; pool: PoolState; onLpChange?: () => void }) {
  const { connection } = useConnection()
  const wallet = useWallet()
  const [tab, setTab] = useState<'add' | 'remove'>('add')
  const [solAmount, setSolAmount] = useState('')
  const [lpAmount, setLpAmount] = useState('')
  const [loading, setLoading] = useState(false)
  const [status, setStatus] = useState('')
  const [lpBalance, setLpBalance] = useState(0)

  // Fetch LP balance
  useEffect(() => {
    if (!wallet.publicKey) return
    const fetchLp = async () => {
      try {
        const [poolPda] = getPoolPda(new PublicKey(mint))
        const [lpMint] = getLpMintPda(poolPda)
        const lpAta = getAssociatedTokenAddressSync(lpMint, wallet.publicKey!, false, TOKEN_2022_PID)
        const bal = await connection.getTokenAccountBalance(lpAta)
        setLpBalance(Number(bal.value.amount))
      } catch {
        setLpBalance(0)
      }
    }
    fetchLp()
  }, [connection, wallet.publicKey, mint, tab])

  // Compute token amount from SOL input (proportional to pool ratio)
  const solNum = parseFloat(solAmount) || 0
  const solLamports = Math.floor(solNum * LAMPORTS_PER_SOL)
  const poolHasLiquidity = pool.solReserve > LAMPORTS_PER_SOL / 100 && pool.tokenReserve > 0
  const tokenRequired = poolHasLiquidity
    ? Math.ceil(solLamports * pool.tokenReserve / pool.solReserve)
    : 0

  const handleAdd = async () => {
    if (!wallet.publicKey || !wallet.sendTransaction || tokenRequired <= 0) return
    setLoading(true)
    setStatus('')
    try {
      const maxSol = Math.ceil(solLamports * 1.05) // 5% buffer
      const { transaction } = await buildAddLiquidityTransaction(connection, {
        provider: wallet.publicKey.toBase58(),
        tokenMint: mint,
        tokenAmount: tokenRequired,
        maxSolAmount: maxSol,
      })
      const sig = await wallet.sendTransaction(transaction, connection)
      await connection.confirmTransaction(sig, 'confirmed')
      setStatus('Liquidity added')
      setSolAmount('')
      onLpChange?.()
    } catch (e: any) {
      setStatus(`Error: ${e.message?.slice(0, 80)}`)
    } finally {
      setLoading(false)
    }
  }

  const handleRemove = async () => {
    if (!wallet.publicKey || !wallet.sendTransaction) return
    const lp = Math.floor(parseFloat(lpAmount) * 1e6)
    if (lp <= 0) return
    setLoading(true)
    setStatus('')
    try {
      const { transaction } = await buildRemoveLiquidityTransaction(connection, {
        provider: wallet.publicKey.toBase58(),
        tokenMint: mint,
        lpAmount: lp,
        minSolOut: 1,
        minTokensOut: 1,
      })
      const sig = await wallet.sendTransaction(transaction, connection)
      await connection.confirmTransaction(sig, 'confirmed')
      setStatus('Liquidity removed')
      setLpAmount('')
      onLpChange?.()
    } catch (e: any) {
      setStatus(`Error: ${e.message?.slice(0, 80)}`)
    } finally {
      setLoading(false)
    }
  }

  const presetBtn = (label: string, onClick: () => void) => (
    <button
      key={label}
      onClick={onClick}
      style={{
        padding: '6px 12px', borderRadius: '6px',
        border: '1px solid var(--border-color)', background: 'var(--surface)',
        color: 'var(--foreground)', fontSize: '13px', fontWeight: 500, cursor: 'pointer',
      }}
      onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--surface-hover)')}
      onMouseLeave={(e) => (e.currentTarget.style.background = 'var(--surface)')}
    >
      {label}
    </button>
  )

  return (
    <div style={{ padding: '20px' }}>
      <div style={{ display: 'flex', gap: '8px', marginBottom: '16px' }}>
        <button
          onClick={() => setTab('add')}
          style={{
            flex: 1, padding: '10px', borderRadius: '8px', border: 'none',
            fontWeight: 600, fontSize: '14px', cursor: 'pointer',
            background: tab === 'add' ? 'var(--accent)' : 'var(--surface)',
            color: tab === 'add' ? '#000' : 'var(--muted)',
          }}
        >
          Add
        </button>
        <button
          onClick={() => setTab('remove')}
          style={{
            flex: 1, padding: '10px', borderRadius: '8px', border: 'none',
            fontWeight: 600, fontSize: '14px', cursor: 'pointer',
            background: tab === 'remove' ? 'var(--danger)' : 'var(--surface)',
            color: tab === 'remove' ? '#000' : 'var(--muted)',
          }}
        >
          Remove
        </button>
      </div>

      {tab === 'add' ? (
        <>
          <div style={{ marginBottom: '12px' }}>
            <label style={{ fontSize: '12px', color: 'var(--muted)', marginBottom: '4px', display: 'block' }}>
              SOL to deposit
            </label>
            <input
              type="number"
              value={solAmount}
              onChange={(e) => setSolAmount(e.target.value)}
              placeholder="0.0"
              className="font-mono"
              style={{
                width: '100%', padding: '12px', borderRadius: '8px',
                border: '1px solid var(--border-color)', background: 'var(--surface)',
                color: 'var(--foreground)', fontSize: '16px', outline: 'none',
              }}
            />
          </div>

          <div style={{ display: 'flex', gap: '6px', marginBottom: '12px' }}>
            {[0.1, 0.5, 1, 5].map((sol) =>
              presetBtn(`${sol} SOL`, () => setSolAmount(sol.toString())),
            )}
          </div>

          {solNum > 0 && (
            <div style={{
              padding: '12px', borderRadius: '8px', background: 'var(--surface)',
              marginBottom: '12px', fontSize: '13px',
            }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '4px' }}>
                <span style={{ color: 'var(--muted)' }}>SOL</span>
                <span className="font-mono">{solNum.toFixed(4)} SOL</span>
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ color: 'var(--muted)' }}>Tokens (proportional)</span>
                <span className="font-mono">{(tokenRequired / 1e6).toFixed(4)}</span>
              </div>
            </div>
          )}

          <div style={{
            padding: '10px 12px', borderRadius: '8px',
            background: 'rgba(6, 182, 212, 0.08)',
            border: '1px solid rgba(6, 182, 212, 0.2)',
            marginBottom: '12px', fontSize: '12px', color: 'var(--accent)',
          }}>
            7.5% of LP tokens are permanently locked in the pool. You receive 92.5%.
          </div>

          <button
            onClick={handleAdd}
            disabled={loading || !wallet.publicKey || solLamports <= 0}
            style={{
              width: '100%', padding: '14px', borderRadius: '8px', border: 'none',
              fontWeight: 600, fontSize: '15px',
              cursor: loading || !wallet.publicKey ? 'not-allowed' : 'pointer',
              background: wallet.publicKey ? 'var(--accent)' : 'var(--surface)',
              color: wallet.publicKey ? '#000' : 'var(--muted)',
              opacity: loading ? 0.6 : 1,
            }}
          >
            {loading ? 'Adding...' : !wallet.publicKey ? 'Connect Wallet' : 'Add Liquidity'}
          </button>
        </>
      ) : (
        <>
          <div style={{ marginBottom: '12px' }}>
            <label style={{ fontSize: '12px', color: 'var(--muted)', marginBottom: '4px', display: 'block' }}>
              LP tokens to burn
              {lpBalance > 0 && (
                <span style={{ float: 'right' }}>
                  Balance: {(lpBalance / 1e6).toFixed(4)}
                </span>
              )}
            </label>
            <input
              type="number"
              value={lpAmount}
              onChange={(e) => setLpAmount(e.target.value)}
              placeholder="0.0"
              className="font-mono"
              style={{
                width: '100%', padding: '12px', borderRadius: '8px',
                border: '1px solid var(--border-color)', background: 'var(--surface)',
                color: 'var(--foreground)', fontSize: '16px', outline: 'none',
              }}
            />
          </div>

          <div style={{ display: 'flex', gap: '6px', marginBottom: '12px' }}>
            {[25, 50, 75, 100].map((pct) =>
              presetBtn(`${pct}%`, () => {
                let amt = Math.floor(lpBalance * pct / 100)
                if (pct === 100) amt = Math.max(amt - 1, 0) // avoid rounding into full drain
                setLpAmount((amt / 1e6).toString())
              }),
            )}
          </div>

          <button
            onClick={handleRemove}
            disabled={loading || !wallet.publicKey || !(parseFloat(lpAmount) > 0)}
            style={{
              width: '100%', padding: '14px', borderRadius: '8px', border: 'none',
              fontWeight: 600, fontSize: '15px',
              cursor: loading || !wallet.publicKey ? 'not-allowed' : 'pointer',
              background: wallet.publicKey ? 'var(--danger)' : 'var(--surface)',
              color: wallet.publicKey ? '#000' : 'var(--muted)',
              opacity: loading ? 0.6 : 1,
            }}
          >
            {loading ? 'Removing...' : !wallet.publicKey ? 'Connect Wallet' : 'Remove Liquidity'}
          </button>
        </>
      )}

      {status && (
        <p style={{ marginTop: '8px', fontSize: '12px', color: status.startsWith('Error') ? 'var(--danger)' : 'var(--success)' }}>
          {status}
        </p>
      )}
    </div>
  )
}
