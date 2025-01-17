import { ethers } from "ethers";
import { abi as ComputePoolABI } from "../abi/ComputePool.json";
import { abi as PrimeNetworkABI } from "../abi/PrimeNetwork.json";
import { abi as ComputeRegistryABI } from "../abi/ComputeRegistry.json";
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

export async function getValidationStatus(computeNodeAddress: string, providerAddress: string): Promise<{ isActive: boolean; isValidated: boolean }> {
    const provider = new ethers.JsonRpcProvider(config.rpcUrl);
    const contract = new ethers.Contract(
        config.contracts.computeRegistry,
        ethers.Interface.from(ComputeRegistryABI),
        provider,
    );

    console.log(computeNodeAddress);
    const result = await contract.getNode(providerAddress, computeNodeAddress);
    console.log(result);

    return {
        isActive: result.isActive,
        isValidated: result.isValidated,
    };
}