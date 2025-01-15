import { ethers } from 'ethers'
import dotenv from 'dotenv'
dotenv.config()

const abi = [
  'function createDomain(string name, address validationContract, string domainParametersURI)',
]

const domainRegistryABI = [
  'function domains(uint256) view returns (tuple(uint256 domainId, string name, address validationLogic, string domainParametersURI, address computePool))',
  'function create(string calldata name, address computePool, address validationContract, string calldata domainParametersURI) external returns (uint256)',
  'function get(uint256) view returns (tuple(uint256 domainId, string name, address validationLogic, string domainParametersURI, address computePool))',
]

const contractABI = [
  'function pools(uint256) public view returns (tuple(uint256 poolId, uint256 domainId, string poolName, address creator, address computeManagerKey, uint256 creationTime, uint256 startTime, uint256 endTime, string poolDataURI, address poolValidationLogic, uint256 totalCompute, uint8 status))',
  'struct PoolInfo { uint256 poolId; uint256 domainId; string poolName; address creator; address computeManagerKey; uint256 creationTime; uint256 startTime; uint256 endTime; string poolDataURI; address poolValidationLogic; uint256 totalCompute; uint8 status; }',
  'struct WorkInterval { uint256 joinTime; uint256 leaveTime; }',
  'function createComputePool(uint256 domainId, address computeManagerKey, string calldata poolName, string calldata poolDataURI) external returns (uint256)',
]

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
    // Try using .get() method instead of .domains()
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
    const domain = await domainRegistry.get(0)
    console.log('Domain name:', domain.name)
    if (domain.name !== 'Decentralized Training') {
      throw new Error('Domain name does not match expected value')
    }

    const poolExists = await computePool.pools(0)
    if (!poolExists) {
      const tx = await computePool.createComputePool(
        domain.domainId,
        process.env.POOL_OWNER_ADDRESS!,
        'Decentralized Training',
        'https://primeintellect.ai/training/params'
      )
      console.log('Transaction sent:', tx.hash)
      await tx.wait()
      console.log('Pool created!')
    } else {
      console.log('Pool already exists:', poolExists)
    }
  }
}

main().catch(console.error)
