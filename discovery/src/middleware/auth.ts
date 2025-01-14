import { Request, Response, NextFunction } from 'express'
import {
  verifyEthereumSignature,
  createSignatureMessage,
} from '../utils/ethereum'

export interface SignedRequest {
  signature: string
  address: string
}

export const verifySignature = async (
  req: Request,
  res: Response,
  next: NextFunction
) => {
  try {
    const { signature, address, ...data } = req.body as SignedRequest
    const nodeId = req.params.nodeId

    if (!signature || !address) {
      res.status(400).json({
        success: false,
        message: 'Missing signature or address',
      })
      return
    }

    // Create message from data
    const message = createSignatureMessage(nodeId, data)

    // Verify signature matches address
    if (!verifyEthereumSignature(message, signature, address)) {
      res.status(401).json({
        success: false,
        message: 'Invalid signature',
      })
      return
    }

    // Check if address matches nodeId
    if (nodeId.toLowerCase() !== address.toLowerCase()) {
      res.status(403).json({
        success: false,
        message: 'Address does not match nodeId',
      })
      return
    }

    next()
  } catch (error) {
    res.status(400).json({
      success: false,
      message: 'Invalid signature',
      error: error instanceof Error ? error.message : 'Unknown error',
    })
  }
}
