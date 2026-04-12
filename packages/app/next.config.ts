import type { NextConfig } from 'next'

const nextConfig: NextConfig = {
  env: {
    NEXT_PUBLIC_RPC_URL: process.env.NEXT_PUBLIC_RPC_URL || 'https://karmen-xgrgn5-fast-mainnet.helius-rpc.com',
    NEXT_PUBLIC_NETWORK: process.env.NEXT_PUBLIC_NETWORK || 'devnet',
  },
}

export default nextConfig
