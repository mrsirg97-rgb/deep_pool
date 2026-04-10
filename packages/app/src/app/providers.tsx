'use client'

import { useMemo } from 'react'
import { ConnectionProvider, WalletProvider } from '@solana/wallet-adapter-react'
import { WalletModalProvider } from '@solana/wallet-adapter-react-ui'

import '@solana/wallet-adapter-react-ui/styles.css'

const RPC_ENDPOINTS: Record<string, string> = {
  devnet: 'https://api.devnet.solana.com',
  mainnet: process.env.NEXT_PUBLIC_RPC_URL || 'https://api.mainnet-beta.solana.com',
}

function getNetwork(): string {
  if (typeof window === 'undefined') return 'devnet'
  return localStorage.getItem('deeppool-network') || 'devnet'
}

export function Providers({ children }: { children: React.ReactNode }) {
  const network = getNetwork()
  const endpoint = RPC_ENDPOINTS[network] || RPC_ENDPOINTS.devnet
  const wallets = useMemo(() => [], [])

  return (
    <ConnectionProvider endpoint={endpoint} config={{ commitment: 'confirmed' }}>
      <WalletProvider wallets={wallets} autoConnect>
        <WalletModalProvider>
          {children}
        </WalletModalProvider>
      </WalletProvider>
    </ConnectionProvider>
  )
}
