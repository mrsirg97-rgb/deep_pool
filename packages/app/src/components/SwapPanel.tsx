'use client'

import { useState, useEffect } from 'react'
import { useConnection, useWallet } from '@solana/wallet-adapter-react'
import { LAMPORTS_PER_SOL, PublicKey } from '@solana/web3.js'
import { getAssociatedTokenAddressSync } from '@solana/spl-token'
import { getSwapQuote, buildSwapTransaction } from 'deeppoolsdk'
import type { PoolState } from 'deeppoolsdk'

const TOKEN_2022_PID = new PublicKey('TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb')
const BUY_PRESETS = [0.25, 0.5, 1, 2]
const SELL_PRESETS = [25, 50, 75, 100] // percentages

const SLIPPAGE_OPTIONS = [0.5, 1, 2, 5]

export function SwapPanel({ mint, pool, onSwap }: { mint: string; pool: PoolState; onSwap?: () => void }) {
  const { connection } = useConnection()
  const wallet = useWallet()
  const [buy, setBuy] = useState(true)
  const [amount, setAmount] = useState('')
  const [slippageBps, setSlippageBps] = useState(100) // 1% default
  const [loading, setLoading] = useState(false)
  const [status, setStatus] = useState('')
  const [tokenBalance, setTokenBalance] = useState(0)

  // Fetch token balance for sell presets
  useEffect(() => {
    if (!wallet.publicKey || buy) return
    const fetchBalance = async () => {
      try {
        const ata = getAssociatedTokenAddressSync(new PublicKey(mint), wallet.publicKey!, false, TOKEN_2022_PID)
        const bal = await connection.getTokenAccountBalance(ata)
        setTokenBalance(Number(bal.value.amount))
      } catch {
        setTokenBalance(0)
      }
    }
    fetchBalance()
  }, [connection, wallet.publicKey, mint, buy])

  const amountNum = parseFloat(amount) || 0
  const amountLamports = buy
    ? Math.floor(amountNum * LAMPORTS_PER_SOL)
    : Math.floor(amountNum * 1e6)

  const quote = amountLamports > 0
    ? getSwapQuote(pool.solReserve, pool.tokenReserve, amountLamports, buy)
    : null

  const handleSwap = async () => {
    if (!wallet.publicKey || !wallet.sendTransaction || amountLamports <= 0) return
    setLoading(true)
    setStatus('')
    try {
      const minOut = quote
        ? Math.floor(quote.amountOut * (10000 - slippageBps) / 10000)
        : 1
      const { transaction } = await buildSwapTransaction(connection, {
        user: wallet.publicKey.toBase58(),
        config: pool.config,
        tokenMint: mint,
        amountIn: amountLamports,
        minimumOut: Math.max(minOut, 1),
        buy,
      })
      const sig = await wallet.sendTransaction(transaction, connection)
      await connection.confirmTransaction(sig, 'confirmed')
      setStatus('Swap confirmed')
      setAmount('')
      onSwap?.()
    } catch (e: any) {
      setStatus(`Error: ${e.message?.slice(0, 80)}`)
    } finally {
      setLoading(false)
    }
  }

  const presetBtn = (label: string, onClick: () => void, active = false) => (
    <button
      key={label}
      onClick={onClick}
      style={{
        padding: '6px 12px',
        borderRadius: '6px',
        border: active ? '1px solid rgba(255,255,255,0.25)' : '1px solid var(--border-color)',
        background: active ? 'var(--surface-hover)' : 'var(--surface)',
        color: 'var(--foreground)',
        fontSize: '13px',
        fontWeight: active ? 700 : 500,
        cursor: 'pointer',
      }}
      onMouseEnter={(e) => (e.currentTarget.style.background = 'var(--surface-hover)')}
      onMouseLeave={(e) => { if (!active) e.currentTarget.style.background = 'var(--surface)' }}
    >
      {label}
    </button>
  )

  return (
    <div style={{ padding: '20px' }}>
      <div style={{ display: 'flex', gap: '8px', marginBottom: '16px' }}>
        <button
          onClick={() => { setBuy(true); setAmount('') }}
          style={{
            flex: 1, padding: '10px', borderRadius: '8px', border: 'none',
            fontWeight: 600, fontSize: '14px', cursor: 'pointer',
            background: buy ? 'var(--success)' : 'var(--surface)',
            color: buy ? '#000' : 'var(--muted)',
          }}
        >
          Buy
        </button>
        <button
          onClick={() => { setBuy(false); setAmount('') }}
          style={{
            flex: 1, padding: '10px', borderRadius: '8px', border: 'none',
            fontWeight: 600, fontSize: '14px', cursor: 'pointer',
            background: !buy ? 'var(--danger)' : 'var(--surface)',
            color: !buy ? '#000' : 'var(--muted)',
          }}
        >
          Sell
        </button>
      </div>

      {/* Slippage */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '6px', marginBottom: '12px' }}>
        <span style={{ fontSize: '11px', color: 'var(--muted)', flexShrink: 0 }}>Slippage</span>
        {SLIPPAGE_OPTIONS.map((pct) =>
          presetBtn(`${pct}%`, () => setSlippageBps(pct * 100), slippageBps === pct * 100)
        )}
      </div>

      {/* Presets */}
      <div style={{ display: 'flex', gap: '6px', marginBottom: '12px' }}>
        {buy
          ? BUY_PRESETS.map((sol) =>
              presetBtn(`${sol} SOL`, () => setAmount(sol.toString())),
            )
          : SELL_PRESETS.map((pct) =>
              presetBtn(`${pct}%`, () => {
                const amt = Math.floor(tokenBalance * pct / 100)
                setAmount((amt / 1e6).toString())
              }),
            )}
      </div>

      <div style={{ marginBottom: '12px' }}>
        <label style={{ fontSize: '12px', color: 'var(--muted)', marginBottom: '4px', display: 'block' }}>
          {buy ? 'SOL amount' : 'Token amount'}
          {!buy && tokenBalance > 0 && (
            <span style={{ float: 'right' }}>
              Balance: {(tokenBalance / 1e6).toFixed(2)}
            </span>
          )}
        </label>
        <input
          type="number"
          value={amount}
          onChange={(e) => setAmount(e.target.value)}
          placeholder="0.0"
          className="font-mono"
          style={{
            width: '100%', padding: '12px', borderRadius: '8px',
            border: '1px solid var(--border-color)', background: 'var(--surface)',
            color: 'var(--foreground)', fontSize: '16px', outline: 'none',
          }}
        />
      </div>

      {quote && quote.amountOut > 0 && (
        <div style={{
          padding: '12px', borderRadius: '8px', background: 'var(--surface)',
          marginBottom: '12px', fontSize: '13px',
        }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '4px' }}>
            <span style={{ color: 'var(--muted)' }}>You receive</span>
            <span className="font-mono">
              {buy
                ? `${(quote.amountOut / 1e6).toFixed(4)} tokens`
                : `${(quote.amountOut / LAMPORTS_PER_SOL).toFixed(6)} SOL`}
            </span>
          </div>
          <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '4px' }}>
            <span style={{ color: 'var(--muted)' }}>Fee (0.25%)</span>
            <span className="font-mono" style={{ color: 'var(--muted)' }}>
              {buy
                ? `${(quote.fee / LAMPORTS_PER_SOL).toFixed(6)} SOL`
                : `${(quote.fee / 1e6).toFixed(4)} tokens`}
            </span>
          </div>
          <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '4px' }}>
            <span style={{ color: 'var(--muted)' }}>Price impact</span>
            <span className="font-mono" style={{ color: quote.priceImpactPercent > 5 ? 'var(--danger)' : 'var(--muted)' }}>
              {quote.priceImpactPercent.toFixed(2)}%
            </span>
          </div>
          <div style={{ display: 'flex', justifyContent: 'space-between' }}>
            <span style={{ color: 'var(--muted)' }}>Min received ({slippageBps / 100}%)</span>
            <span className="font-mono">
              {buy
                ? `${(Math.floor(quote.amountOut * (10000 - slippageBps) / 10000) / 1e6).toFixed(4)} tokens`
                : `${(Math.floor(quote.amountOut * (10000 - slippageBps) / 10000) / LAMPORTS_PER_SOL).toFixed(6)} SOL`}
            </span>
          </div>
        </div>
      )}

      <button
        onClick={handleSwap}
        disabled={loading || !wallet.publicKey || amountLamports <= 0}
        style={{
          width: '100%', padding: '14px', borderRadius: '8px', border: 'none',
          fontWeight: 600, fontSize: '15px',
          cursor: loading || !wallet.publicKey ? 'not-allowed' : 'pointer',
          background: wallet.publicKey ? (buy ? 'var(--success)' : 'var(--danger)') : 'var(--surface)',
          color: wallet.publicKey ? '#000' : 'var(--muted)',
          opacity: loading ? 0.6 : 1,
        }}
      >
        {loading ? 'Swapping...' : !wallet.publicKey ? 'Connect Wallet' : buy ? 'Buy' : 'Sell'}
      </button>

      {status && (
        <p style={{ marginTop: '8px', fontSize: '12px', color: status.startsWith('Error') ? 'var(--danger)' : 'var(--success)' }}>
          {status}
        </p>
      )}
    </div>
  )
}
