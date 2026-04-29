import type { NextConfig } from 'next'

const nextConfig: NextConfig = {
  // Standalone output bundles only the deps the app actually imports into
  // .next/standalone/, which is what the Docker runtime stage copies.
  // Without this Next emits a normal build that needs a full node_modules
  // tree at runtime.
  output: 'standalone',
  env: {
    NEXT_PUBLIC_RPC_URL: process.env.NEXT_PUBLIC_RPC_URL || 'https://karmen-xgrgn5-fast-mainnet.helius-rpc.com',
    NEXT_PUBLIC_NETWORK: process.env.NEXT_PUBLIC_NETWORK || 'devnet',
  },
}

export default nextConfig
