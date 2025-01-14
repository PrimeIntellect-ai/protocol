import express, { Request, Response, NextFunction } from 'express'
import helmet from 'helmet'
import morgan from 'morgan'
import { z } from 'zod'
import { verifySignature } from './middleware/auth'
import { redis } from './config/redis'
// Type definitions
interface ApiResponse<T> {
  success: boolean
  message: string
  data?: T
  error?: string
}

// Validation schemas
const NodeSchema = z.object({
  ipAddress: z.string().ip(),
  port: z.number().int().min(1).max(65535),
  capacity: z.number().int().min(0).optional().default(0),
})

type Node = z.infer<typeof NodeSchema>

// Express app setup
const app = express()
app.use(express.json())
app.use(helmet())
app.use(morgan('combined'))

// Register/update a node
app.put<{ nodeId: string }>(
  '/nodes/:nodeId',
  verifySignature,
  async (
    req: Request,
    res: Response<ApiResponse<Node & { nodeId: string }>>,
    next: NextFunction
  ) => {
    try {
      const nodeId = req.params.nodeId
      const result = NodeSchema.safeParse(req.body)

      if (!result.success) {
        res.status(400).json({
          success: false,
          message: 'Invalid request body',
          error: result.error.errors.map((e) => e.message).join(', '),
        })
        return
      }

      const node: Node = result.data
      await redis.hset(
        `node:${nodeId}`,
        'ipAddress',
        node.ipAddress,
        'port',
        node.port,
        'capacity',
        node.capacity,
        'lastSeen',
        Date.now()
      )

      await redis.expire(`node:${nodeId}`, 300)

      res.status(200).json({
        success: true,
        message: 'Node registered successfully',
        data: {
          nodeId,
          ...node,
        },
      })
    } catch (error) {
      next(error)
    }
  }
)

// Get specific node
app.get<{ nodeId: string }>(
  '/nodes/:nodeId',
  async (
    req: Request,
    res: Response<ApiResponse<Node>>,
    next: NextFunction
  ) => {
    try {
      const node = await redis.hgetall(`node:${req.params.nodeId}`)

      if (!node.ipAddress) {
        res.status(404).json({
          success: false,
          message: 'Node not found',
        })
        return
      }

      res.json({
        success: true,
        message: 'Node retrieved successfully',
        data: {
          ipAddress: node.ipAddress,
          port: parseInt(node.port),
          capacity: parseInt(node.capacity),
        },
      })
    } catch (error) {
      next(error)
    }
  }
)

// List all nodes
app.get(
  '/nodes',
  async (
    req: Request,
    res: Response<ApiResponse<Node[]>>,
    next: NextFunction
  ) => {
    try {
      const keys = await redis.keys('node:*')
      const nodes = await Promise.all(
        keys.map(async (key) => {
          const node = await redis.hgetall(key)
          return {
            id: key.replace('node:', ''),
            ipAddress: node.ipAddress,
            port: parseInt(node.port),
            capacity: parseInt(node.capacity),
          } as Node
        })
      )

      res.json({
        success: true,
        message: 'Nodes retrieved successfully',
        data: nodes,
      })
    } catch (error) {
      next(error)
    }
  }
)

// Error handling middleware
app.use(
  (
    error: Error,
    req: Request,
    res: Response<ApiResponse<never>>,
    next: NextFunction
  ) => {
    console.error('Unhandled error:', error)
    res.status(500).json({
      success: false,
      message: 'Internal server error',
      error: error.message,
    })
  }
)

const port = process.env.PORT || 8089
if (require.main === module) {
  app.listen(port, () => console.log(`Discovery service running on ${port}`))
}

export { app, redis }
