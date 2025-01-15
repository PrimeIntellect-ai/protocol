import { ethers } from "ethers";
import { abi as ComputePoolABI } from "../abi/ComputePool.json";
import { abi as PrimeNetworkABI } from "../abi/PrimeNetwork.json";
import { config } from "../config/environment";

export async function isComputePoolOwner(
  poolId: number,
  verifiedAddress: string,
): Promise<boolean> {
  const provider = new ethers.JsonRpcProvider(config.rpcUrl);
  const contract = new ethers.Contract(
    config.contracts.computePool,
    ethers.Interface.from(ComputePoolABI),
    provider,
  );

  try {
    const pool = await contract.getComputePool(poolId);
    return (
      verifiedAddress.toLowerCase() === pool.creator.toLowerCase() ||
      verifiedAddress.toLowerCase() === pool.computeManagerKey.toLowerCase()
    );
  } catch (error) {
    console.error(`Error checking compute access for pool ${poolId}:`, error);
    return false;
  }
}

export async function isValidator(address: string): Promise<boolean> {
  const provider = new ethers.JsonRpcProvider(config.rpcUrl);
  const contract = new ethers.Contract(
    config.contracts.primeNetwork,
    ethers.Interface.from(PrimeNetworkABI),
    provider,
  );

  const validatorRole = await contract.VALIDATOR_ROLE();

  const result = await contract.hasRole(validatorRole, address);
  return result;
}
