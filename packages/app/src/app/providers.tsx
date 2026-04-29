'use client'

import { useMemo } from 'react'
import { ConnectionProvider, WalletProvider } from '@solana/wallet-adapter-react'
import { WalletModalProvider } from '@solana/wallet-adapter-react-ui'

import '@solana/wallet-adapter-react-ui/styles.css'
import { IndexerBusProvider } from '@/lib/bus'

const RPC_ENDPOINTS: Record<string, string> = {
  devnet: 'https://eula-l0ihfg-fast-devnet.helius-rpc.com',
  mainnet: 'https://karmen-xgrgn5-fast-mainnet.helius-rpc.com',
}

const getNetwork = () => {
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
          <IndexerBusProvider>{children}</IndexerBusProvider>
        </WalletModalProvider>
      </WalletProvider>
    </ConnectionProvider>
  )
}
