import { BorshCoder, Idl } from '@coral-xyz/anchor'
import { Connection, PublicKey } from '@solana/web3.js'
import bs58 from 'bs58'
import idl from './deep_pool.json'
import { PROGRAM_ID } from './constants'

const coder = new BorshCoder(idl as unknown as Idl)

// Anchor emit_cpi! prefix: little-endian u64 0x1d9acb512ea545e4.
// Constant across every Anchor-emitted event regardless of event name.
const EVENT_IX_TAG = Buffer.from([228, 69, 165, 46, 81, 203, 154, 29])

const EVENT_DISCRIMINATORS: Record<string, Buffer> = {
  PoolCreated: Buffer.from([202, 44, 41, 88, 104, 220, 157, 82]),
  LiquidityAdded: Buffer.from([154, 26, 221, 108, 238, 64, 217, 161]),
  LiquidityRemoved: Buffer.from([225, 105, 216, 39, 124, 116, 169, 189]),
  SwapExecuted: Buffer.from([150, 166, 26, 225, 28, 89, 38, 79]),
}

export interface DecodedEvent<T = any> {
  name: string
  data: T
}

export const parseEvents = async (
  connection: Connection,
  signature: string,
): Promise<DecodedEvent[]> => {
  const tx = await connection.getTransaction(signature, {
    commitment: 'confirmed',
    maxSupportedTransactionVersion: 0,
  })
  if (!tx?.meta?.innerInstructions) return []

  const msg = tx.transaction.message as any
  const accountKeys: PublicKey[] = msg.staticAccountKeys ?? msg.accountKeys
  const events: DecodedEvent[] = []

  for (const inner of tx.meta.innerInstructions) {
    for (const ix of inner.instructions) {
      const programId = accountKeys[ix.programIdIndex]
      if (!programId.equals(PROGRAM_ID)) continue

      const data = Buffer.from(bs58.decode(ix.data))
      if (data.length < 16) continue
      if (!data.subarray(0, 8).equals(EVENT_IX_TAG)) continue

      const disc = data.subarray(8, 16)
      const payload = data.subarray(16)

      for (const [name, expected] of Object.entries(EVENT_DISCRIMINATORS)) {
        if (disc.equals(expected)) {
          const decoded = (coder.types as any).decode(name, payload)
          events.push({ name, data: decoded })
          break
        }
      }
    }
  }

  return events
}
