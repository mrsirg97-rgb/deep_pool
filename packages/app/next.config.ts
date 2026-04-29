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
    // Optional. When set, the SDK's pool getters route through this indexer
    // first and fall back to RPC on any failure. Docker compose bakes
    // http://localhost:8080 into the build via build args; local `next dev`
    // can leave this unset for pure RPC reads.
    NEXT_PUBLIC_INDEXER_URL: process.env.NEXT_PUBLIC_INDEXER_URL || '',
  },
}

export default nextConfig
