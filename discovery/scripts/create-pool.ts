/* eslint-disable */
import { ethers } from 'ethers'
import dotenv from 'dotenv'
dotenv.config()

const abi = [
  'function createDomain(string name, address validationContract, string domainParametersURI)',
]

const domainRegistryABI = [
  'function create(string calldata name, address computePool, address validationContract, string calldata domainParametersURI) external returns (uint256)',
  'function get(uint256) view returns (tuple(uint256 domainId, string name, address validationLogic, string domainParametersURI, address computePool))',
]

const contractABI = [
  'function getComputePool(uint256 poolId) external view returns (tuple(uint256 poolId, uint256 domainId, string poolName, address creator, address computeManagerKey, uint256 creationTime, uint256 startTime, uint256 endTime, string poolDataURI, address poolValidationLogic, uint256 totalCompute, uint8 status))',
  'function pools(uint256) public view returns (tuple(uint256 poolId, uint256 domainId, string poolName, address creator, address computeManagerKey, uint256 creationTime, uint256 startTime, uint256 endTime, string poolDataURI, address poolValidationLogic, uint256 totalCompute, uint8 status))',
  'struct PoolInfo { uint256 poolId; uint256 domainId; string poolName; address creator; address computeManagerKey; uint256 creationTime; uint256 startTime; uint256 endTime; string poolDataURI; address poolValidationLogic; uint256 totalCompute; uint8 status; }',
  'struct WorkInterval { uint256 joinTime; uint256 leaveTime; }',
  'function createComputePool(uint256 domainId, address computeManagerKey, string poolName, string poolDataURI) external returns (uint256)'
];

async function main() {
  const provider = new ethers.JsonRpcProvider(process.env.RPC_URL)
  const wallet = new ethers.Wallet(process.env.PRIVATE_KEY_FEDERATOR!, provider)

  console.log('Domain Registry Address:', process.env.DOMAIN_REGISTRY_ADDRESS)
  console.log('RPC URL:', process.env.RPC_URL)

  const primeNetwork = new ethers.Contract(
    process.env.PRIME_NETWORK_ADDRESS!,
    abi,
    wallet
  )
  const computePool = new ethers.Contract(
    process.env.COMPUTE_POOL_ADDRESS!,
    contractABI,
    wallet
  )

  const domainRegistry = new ethers.Contract(
    process.env.DOMAIN_REGISTRY_ADDRESS!,
    domainRegistryABI,
    wallet
  )

  // Check if domain exists
  console.log('Checking if domain exists...')
  let domain
  try {
    domain = await domainRegistry.get(0)
    console.log('Domain:', domain)
  } catch (error) {
    console.log('Error retrieving domain:', error)
  }

  if (!domain) {
    // Create domain using domain registry's create method
    const tx = await primeNetwork.createDomain(
      'Decentralized Training',
      ethers.ZeroAddress,
      'https://primeintellect.ai/training/params'
    )

    console.log('Transaction sent:', tx.hash)
    await tx.wait()
    console.log('Domain created!')

    // Verify domain was created correctly
    domain = await domainRegistry.get(0)
    console.log('Domain name:', domain.name)
    if (domain.name !== 'Decentralized Training') {
      throw new Error('Domain name does not match expected value')
    }
  }
  const pool = await computePool.getComputePool(0)
  console.log('Pool:', pool)

  // Bigger bug here in script - pool is created by federator
  return;
  const poolTx = await computePool.createComputePool(
    domain.domainId,
    process.env.POOL_OWNER_ADDRESS!,
    'Decentralized Training Pool',
    'https://primeintellect.ai/training/params'
  );
  console.log('Compute Pool creation tx:', poolTx.hash);
  await poolTx.wait();
}

main().catch(console.error)
