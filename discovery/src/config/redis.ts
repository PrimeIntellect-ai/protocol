import Redis from 'ioredis'

export const redis = new Redis({
  host: process.env.REDIS_HOST || 'localhost',
  port: Number(process.env.REDIS_PORT) || 6379,
})

// Add a health check function
export const checkRedisConnection = async (): Promise<boolean> => {
  try {
    await redis.ping()
    return true
  } catch {
    return false
  }
}
