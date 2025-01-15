import express, { Request, Response, NextFunction } from 'express'
import {
  verifyPoolOwner,
  verifySignature,
  verifyPrimeValidator,
} from '../middleware/auth'
import {
  registerNode,
  getNode,
  getAllNodes,
  getNodesForPool,
} from '../service/nodes.service'
import { ApiResponse } from '../schemas/apiResponse.schema'
import { ComputeNode } from '../schemas/node.schema'

const router = express.Router()

router.put<{ address: string }, ApiResponse<{ address: string }>>(
  '/nodes/:address',
  verifySignature,
  async (
    req: Request,
    res: Response<ApiResponse<{ address: string }>>,
    next: NextFunction
  ) => {
    try {
      const address = req.params.address
      const nodeData = req.body
      const response = await registerNode(address, nodeData)

      res.status(200).json({
        success: true,
        message: response.message,
        data: {
          address: address,
          ...nodeData,
        },
      })
    } catch (error: unknown) {
      if (
        error instanceof Error &&
        error.message === 'Please wait 5 minutes between updates'
      ) {
        res.status(429).json({
          success: false,
          message: error.message,
        })
      } else {
        next(error)
      }
    }
  }
)

// Get specific node
router.get<{ address: string }, ApiResponse<ComputeNode>>(
  '/nodes/single/:address',
  verifySignature,
  async (
    req: Request,
    res: Response<ApiResponse<ComputeNode>>,
    next: NextFunction
  ) => {
    try {
      const nodeResult = await getNode(req.params.address)

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

// List all nodes for validator
router.get<{}, ApiResponse<ComputeNode[]>>(
  '/nodes/validator',
  verifySignature,
  verifyPrimeValidator,
  async (req, res, next) => {
    try {
      const nodes = await getAllNodes()

      res.status(200).json({
        success: true,
        message: 'Nodes retrieved successfully for validator',
        data: nodes,
      })
    } catch (error) {
      next(error)
    }
  }
)

// List all nodes for a specific compute pool
router.get<{ computePoolId: string }, ApiResponse<ComputeNode[]>>(
  '/nodes/pool/:computePoolId',
  verifySignature,
  verifyPoolOwner,
  async (
    req: Request<{ computePoolId: string }>,
    res: Response<ApiResponse<ComputeNode[]>>,
    next: NextFunction
  ) => {
    try {
      const computePoolId = parseInt(req.params.computePoolId)
      const nodes = await getNodesForPool(computePoolId)

      res.json({
        success: true,
        message: 'Nodes retrieved successfully for compute pool',
        data: nodes,
      } as ApiResponse<ComputeNode[]>)
    } catch (error) {
      next(error)
    }
  }
)

export default router
