import { redis } from '../src/config/redis';
import { beforeAll, beforeEach, afterAll } from '@jest/globals';

beforeAll(async () => {
    await redis.ping() // Test connection
  })

beforeEach(async () => {
  // Clear Redis database before each test
  await redis.flushall()
})

afterAll(async () => {
  // Close Redis connection after all tests
  await redis.quit()
})