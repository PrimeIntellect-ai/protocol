import { ethers } from 'ethers'

export const verifyEthereumSignature = (
  message: string,
  signature: string,
  expectedAddress: string
): boolean => {
  try {
    const recoveredAddress = ethers.verifyMessage(message, signature)
    return recoveredAddress.toLowerCase() === expectedAddress.toLowerCase()
  } catch (error) {
    return false
  }
}

export const createSignatureMessage = (
  nodeId: string,
  data: Record<string, any>
): string => {
  // Create deterministic message by sorting keys
  return nodeId + JSON.stringify(data, Object.keys(data).sort())
}
