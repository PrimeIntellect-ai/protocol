import { ethers } from "ethers";

export const verifyEthereumSignature = (
  message: string,
  signature: string,
  expectedAddress: string,
): boolean => {
  if (!ethers.isHexString(signature)) {
    return false;
  }

  try {
    const recoveredAddress = ethers.verifyMessage(message, signature);
    if (!ethers.isAddress(recoveredAddress)) {
      return false;
    }
    return recoveredAddress.toLowerCase() === expectedAddress.toLowerCase();
  } catch {
    return false;
  }
};
