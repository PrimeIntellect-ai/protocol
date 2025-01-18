import { ethers } from "ethers";
import { abi as ComputePoolABI } from "../abi/ComputePool.json";
import { abi as ComputeRegistryABI } from "../abi/ComputeRegistry.json";
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
    console.log("Pool: ", pool);
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
export async function getValidationStatus(
  computeNodeAddress: string,
  providerAddress: string,
): Promise<{ isActive: boolean | null; isValidated: boolean | null }> {
  if (!config.contracts.computeRegistry) {
    console.error("Invalid contract address for computeRegistry");
    throw new Error("Invalid contract address");
  }

  const provider = new ethers.JsonRpcProvider(config.rpcUrl);
  const contract = new ethers.Contract(
    config.contracts.computeRegistry,
    ethers.Interface.from(ComputeRegistryABI),
    provider,
  );

  try {
    const result = await contract.getNode(providerAddress, computeNodeAddress);
    return {
      isActive: result.isActive,
      isValidated: result.isValidated,
    };
  } catch (error) {
    console.error(
      `Error fetching validation status for node ${computeNodeAddress}:`,
      error,
    );
    return {
      isActive: null,
      isValidated: null,
    };
  }
}
