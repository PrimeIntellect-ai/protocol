import { ethers } from "ethers";

export const verifyEthereumSignature = (
  message: string,
  signature: string,
  expectedAddress: string,
): boolean => {
  console.log("Signature: ", signature);
  if (!ethers.isHexString(signature)) {
    console.log("Signature is not a valid hex string");
    return false;
  }

  console.log("Message: ", message);
  console.log("Signature: ", signature);
  console.log("Expected Address: ", expectedAddress);

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
