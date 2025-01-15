import dotenv from 'dotenv'
dotenv.config()

export const config = {
  rpcUrl: process.env.RPC_URL!,
  networkId: Number(process.env.NETWORK_ID!),
  contracts: {
    primeNetwork: process.env.PRIME_NETWORK_ADDRESS!,
    computePool: process.env.COMPUTE_POOL_ADDRESS!,
  },
} as const

// Validate all required environment variables are present
Object.entries(config).forEach(([key, value]) => {
  if (value === undefined) {
    throw new Error(`Missing environment variable: ${key}`)
  }
})
