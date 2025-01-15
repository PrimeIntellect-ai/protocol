import express, { Request, Response, NextFunction } from 'express'
import helmet from 'helmet'
import morgan from 'morgan'
import { z } from 'zod'
import {
  verifyPoolOwner,
  verifySignature,
  checkComputeAccess,
} from './middleware/auth'
import { redis } from './config/redis'
import { isAddress } from 'ethers'
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
  computePoolId: z.number().int().min(0).optional(),
})

type Node = z.infer<typeof NodeSchema>

// Express app setup
const app = express()
app.use(express.json())
app.use(helmet())
app.use(morgan('combined'))

// Register/update a node
app.put<{ address: string }>(
  '/nodes/:address',
  verifySignature,
  async (
    req: Request,
    res: Response<ApiResponse<Node & { address: string }>>,
    next: NextFunction
  ) => {
    try {
      const address = req.params.address
      if (address !== req.headers['x-verified-address']) {
        res.status(401).json({
          success: false,
          message: 'Invalid signature',
        })
        return
      }
      const result = NodeSchema.safeParse(req.body)

      if (!result.success) {
        res.status(400).json({
          success: false,
          message: 'Invalid request body',
          error: result.error.errors.map((e) => e.message).join(', '),
        })
        return
      }
      const nodeData = await redis.get(`node:${address}`)
      const lastSeen = nodeData ? JSON.parse(nodeData).lastSeen : null
      const now = Date.now()
      if (lastSeen !== null && now - Number(lastSeen) < 5 * 60 * 1000) {
        res.status(429).json({
          success: false,
          message: 'Please wait 5 minutes between updates',
        })
        return
      }

      const node: Node = result.data
      // Store as JSON string to preserve types
      await redis.set(
        `node:${address}`,
        JSON.stringify({
          ...node,
          lastSeen: now,
        })
      )

      res.status(200).json({
        success: true,
        message: 'Node registered successfully',
        data: {
          ipAddress: node.ipAddress,
          port: node.port,
          capacity: node.capacity,
          computePoolId: node.computePoolId,
          address: address,
        },
      })
    } catch (error) {
      next(error)
    }
  }
)

// Get specific node
app.get<{ address: string }>(
  '/nodes/:address',
  async (
    req: Request,
    res: Response<ApiResponse<Node>>,
    next: NextFunction
  ) => {
    try {
      const rawNode = await redis.get(`node:${req.params.address}`)

      if (!rawNode) {
        res.status(404).json({
          success: false,
          message: 'Node not found',
        })
        return
      }

      // Parse the JSON string from Redis
      const nodeResult = NodeSchema.safeParse(JSON.parse(rawNode))

      if (!nodeResult.success) {
        res.status(500).json({
          success: false,
          message: 'Error parsing node data',
          error: nodeResult.error.errors.map((e) => e.message).join(', '),
        })
        return
      }

      res.json({
        success: true,
        message: 'Node retrieved successfully',
        data: nodeResult.data,
      })
    } catch (error) {
      next(error)
    }
  }
)

// List all nodes
app.get<{ computePoolId: string }>(
  '/nodes/pool/:computePoolId',
  verifySignature,
  verifyPoolOwner,
  async (
    req: Request<{ computePoolId: string }>,
    res: Response<ApiResponse<Node[]>>,
    next: NextFunction
  ) => {
    try {
      const computePoolId = parseInt(req.params.computePoolId)

      const keys = await redis.keys('node:*')
      const nodes = await Promise.all(
        keys.map(async (key) => {
          const rawNode = await redis.get(key)
          if (!rawNode) return null

          const node = JSON.parse(rawNode)
          if (node.computePoolId === computePoolId) {
            return {
              id: key.replace('node:', ''),
              ...node,
            } as Node
          }
          return null
        })
      )

      const filteredNodes = nodes.filter((node): node is Node => node !== null)

      res.json({
        success: true,
        message: 'Nodes retrieved successfully',
        data: filteredNodes,
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
