import { PublicKey } from '@solana/web3.js'
import { EVENT_AUTHORITY_SEED, LP_MINT_SEED, POOL_SEED, PROGRAM_ID, VAULT_SEED } from './constants'

export const getPoolPda = (config: PublicKey, tokenMint: PublicKey): [PublicKey, number] =>
  PublicKey.findProgramAddressSync([POOL_SEED, config.toBuffer(), tokenMint.toBuffer()], PROGRAM_ID)

export const getVaultPda = (pool: PublicKey): [PublicKey, number] =>
  PublicKey.findProgramAddressSync([VAULT_SEED, pool.toBuffer()], PROGRAM_ID)

export const getLpMintPda = (pool: PublicKey): [PublicKey, number] =>
  PublicKey.findProgramAddressSync([LP_MINT_SEED, pool.toBuffer()], PROGRAM_ID)

export const getEventAuthorityPda = (): [PublicKey, number] =>
  PublicKey.findProgramAddressSync([EVENT_AUTHORITY_SEED], PROGRAM_ID)
