import { Request, Response, NextFunction } from 'express'
import { verifyEthereumSignature } from '../utils/ethereum'
import { ethers } from 'ethers'
import { config } from '../config/environment'
import { abi as ComputePoolABI } from '../abi/ComputePool.json'

/**
 * Middleware to verify the Ethereum signature of a request.
 *
 * The user must create a signature by signing a message that includes the request URL and the payload.
 * The address of the user should be included in the request headers as 'x-eth-address'.
 *
 * Example of creating a signature:
 * 1. Construct the message: `const message = url + JSON.stringify(payload);`
 * 2. Sign the message using the user's private key: `const signature = await wallet.signMessage(message);`
 *
 * The request should include:
 * - The signature and address in the request headers.
 */
export const verifySignature = async (
  req: Request,
  res: Response,
  next: NextFunction
) => {
  try {
    const address = req.headers['x-eth-address'] as string
    const signature = req.headers['x-signature'] as string

    if (!signature || !address) {
      res.status(400)
      res.json({
        success: false,
        message: 'Missing signature or address',
      })
      return
    }

    const payload =
      req.method === 'POST' || req.method === 'PUT'
        ? JSON.stringify({ ...req.body }, Object.keys(req.body).sort())
        : ''

    const url = req.originalUrl.split('?')[0] // Remove query parameters if any
    console.log(url)
    const message = url + payload

    if (!verifyEthereumSignature(message, signature, address)) {
      res.status(401)
      res.json({
        success: false,
        message: 'Invalid signature',
      })
      return
    }

    // Additional security check: Ensure the address is a valid Ethereum address
    if (!ethers.isAddress(address)) {
      res.status(400)
      res.json({
        success: false,
        message: 'Invalid Ethereum address',
      })
      return
    }

    // Set the verified address in the request header for use in the next middleware
    req.headers['x-verified-address'] = address
    next()
  } catch (error) {
    res.status(400)
    res.json({
      success: false,
      message: 'Invalid signature',
      error: error instanceof Error ? error.message : 'Unknown error',
    })
    return
  }
}

export async function checkComputeAccess(
  poolId: number,
  verifiedAddress: string
): Promise<boolean> {
  const provider = new ethers.JsonRpcProvider(config.rpcUrl)
  const contract = new ethers.Contract(
    config.contracts.computePool,
    ethers.Interface.from(ComputePoolABI),
    provider
  )

  try {
    const pool = await contract.getComputePool(poolId)
    return (
      verifiedAddress.toLowerCase() === pool.creator.toLowerCase() ||
      verifiedAddress.toLowerCase() === pool.computeManagerKey.toLowerCase()
    )
  } catch (error) {
    console.error(`Error checking compute access for pool ${poolId}:`, error)
    return false
  }
}

export const verifyPoolOwner = async (
  req: Request,
  res: Response,
  next: NextFunction
) => {
  const computePoolId = parseInt(
    req.params.computePoolId || req.body.computePoolId
  )
  if (isNaN(computePoolId)) {
    res.status(400)
    res.json({
      success: false,
      message: 'Invalid or missing compute pool ID',
    })
    return
  }

  const verifiedAddress = req.headers['x-verified-address'] as string
  if (!verifiedAddress) {
    res.status(401)
    res.json({
      success: false,
      message: 'Missing verified address',
    })
    return
  }

  const isPoolOwner = await checkComputeAccess(computePoolId, verifiedAddress)
  if (!isPoolOwner) {
    res.status(403)
    res.json({
      success: false,
      message: 'Unauthorized',
    })
    return
  }

  next()
}
