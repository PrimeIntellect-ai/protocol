import Redis from 'ioredis'
import { config } from './environment'

export const redis = new Redis({
  host: config.redis.host,
  port: config.redis.port,
  password: config.redis.password,
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
