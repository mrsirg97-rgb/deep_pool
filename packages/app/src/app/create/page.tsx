'use client'

import { useState } from 'react'
import { useRouter } from 'next/navigation'
import { useConnection, useWallet } from '@solana/wallet-adapter-react'
import { LAMPORTS_PER_SOL } from '@solana/web3.js'
import { buildCreatePoolTransaction } from 'deeppoolsdk'
import { Header } from '@/components/Header'

export default function CreatePage() {
  const { connection } = useConnection()
  const wallet = useWallet()
  const router = useRouter()
  const [tokenMint, setTokenMint] = useState('')
  const [tokenAmount, setTokenAmount] = useState('')
  const [solAmount, setSolAmount] = useState('')
  const [loading, setLoading] = useState(false)
  const [status, setStatus] = useState('')

  const handleCreate = async () => {
    if (!wallet.publicKey || !wallet.sendTransaction) return
    if (!tokenMint || !tokenAmount || !solAmount) return
    setLoading(true)
    setStatus('')
    try {
      // Check if pool already exists
      const { getPool } = await import('deeppoolsdk')
      const existing = await getPool(connection, tokenMint, wallet.publicKey!.toBase58())
      if (existing) {
        setStatus(`Pool already exists for this token`)
        setLoading(false)
        return
      }
      const { transaction, pool } = await buildCreatePoolTransaction(connection, {
        creator: wallet.publicKey.toBase58(),
        config: wallet.publicKey.toBase58(),
        tokenMint,
        initialTokenAmount: Math.floor(parseFloat(tokenAmount) * 1e6),
        initialSolAmount: Math.floor(parseFloat(solAmount) * LAMPORTS_PER_SOL),
      })
      // Simulate first to get logs on failure
      const sim = await connection.simulateTransaction(transaction)
      if (sim.value.err) {
        console.error('Simulation failed:', sim.value.err, sim.value.logs)
        setStatus(`Simulation failed: ${sim.value.logs?.slice(-3).join(' | ')}`)
        setLoading(false)
        return
      }

      const sig = await wallet.sendTransaction(transaction, connection)
      await connection.confirmTransaction(sig, 'confirmed')
      setStatus(`Pool created: ${pool.slice(0, 8)}...`)
      setTimeout(() => router.push(`/pool/${tokenMint}`), 1500)
    } catch (e: any) {
      console.error('Create pool error:', e)
      setStatus(`Error: ${e.message?.slice(0, 200)}`)
    } finally {
      setLoading(false)
    }
  }

  return (
    <>
      <Header />
      <main style={{ maxWidth: '500px', margin: '0 auto', padding: '24px' }}>
        <h1 style={{ fontSize: '24px', fontWeight: 700, marginBottom: '24px' }}>Create Pool</h1>

        <div style={{
          borderRadius: '12px',
          border: '1px solid var(--border-color)',
          padding: '24px',
        }}>
          <div style={{ marginBottom: '16px' }}>
            <label style={{ fontSize: '12px', color: 'var(--muted)', marginBottom: '4px', display: 'block' }}>
              Token Mint (Token-2022)
            </label>
            <input
              value={tokenMint}
              onChange={(e) => setTokenMint(e.target.value)}
              placeholder="Token mint address"
              className="font-mono"
              style={{
                width: '100%', padding: '12px', borderRadius: '8px',
                border: '1px solid var(--border-color)', background: 'var(--surface)',
                color: 'var(--foreground)', fontSize: '13px', outline: 'none',
              }}
            />
          </div>

          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '12px', marginBottom: '16px' }}>
            <div>
              <label style={{ fontSize: '12px', color: 'var(--muted)', marginBottom: '4px', display: 'block' }}>
                Initial tokens
              </label>
              <input
                type="number"
                value={tokenAmount}
                onChange={(e) => setTokenAmount(e.target.value)}
                placeholder="0.0"
                className="font-mono"
                style={{
                  width: '100%', padding: '12px', borderRadius: '8px',
                  border: '1px solid var(--border-color)', background: 'var(--surface)',
                  color: 'var(--foreground)', fontSize: '16px', outline: 'none',
                }}
              />
            </div>
            <div>
              <label style={{ fontSize: '12px', color: 'var(--muted)', marginBottom: '4px', display: 'block' }}>
                Initial SOL
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
          </div>

          {tokenAmount && solAmount && parseFloat(tokenAmount) > 0 && parseFloat(solAmount) > 0 && (
            <div style={{ padding: '12px', borderRadius: '8px', background: 'var(--surface)', marginBottom: '16px', fontSize: '13px' }}>
              <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                <span style={{ color: 'var(--muted)' }}>Initial price</span>
                <span className="font-mono">
                  {(parseFloat(solAmount) / parseFloat(tokenAmount)).toFixed(9)} SOL/token
                </span>
              </div>
            </div>
          )}

          <button
            onClick={handleCreate}
            disabled={loading || !wallet.publicKey || !tokenMint}
            style={{
              width: '100%', padding: '14px', borderRadius: '8px', border: 'none',
              fontWeight: 600, fontSize: '15px',
              cursor: loading || !wallet.publicKey ? 'not-allowed' : 'pointer',
              background: wallet.publicKey ? 'var(--accent)' : 'var(--surface)',
              color: wallet.publicKey ? '#000' : 'var(--muted)',
              opacity: loading ? 0.6 : 1,
            }}
          >
            {loading ? 'Creating...' : !wallet.publicKey ? 'Connect Wallet' : 'Create Pool'}
          </button>

          {status && (
            <p style={{ marginTop: '8px', fontSize: '12px', color: status.startsWith('Error') ? 'var(--danger)' : 'var(--success)' }}>
              {status}
            </p>
          )}
        </div>
      </main>
    </>
  )
}
