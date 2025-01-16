import { redis } from "../config/redis";
import { ComputeNodeSchema, type ComputeNode } from "../schemas/node.schema";

// Function to register a node
export const registerNode = async (address: string, nodeData: ComputeNode) => {
  if (!address || typeof address !== "string") {
    throw new Error("Invalid address");
  }
  const result = ComputeNodeSchema.safeParse(nodeData);
  if (!result.success) {
    throw new Error("Parsing error: Invalid request body");
  }

  const lastSeen = await getLastSeen(address);
  const now = Date.now();
  if (lastSeen !== null && now - lastSeen < 5 * 60 * 1000) {
    throw new Error("Please wait 5 minutes between updates");
  }

  await redis.set(
    `node:${address}`,
    JSON.stringify({
      ...result.data,
      lastSeen: now,
    }),
  );

  return {
    message: "Node registered successfully",
  };
};

// Function to get last seen timestamp
const getLastSeen = async (address: string) => {
  if (!address || typeof address !== "string") {
    throw new Error("Invalid address");
  }

  const nodeData = await redis.get(`node:${address}`);
  return nodeData ? JSON.parse(nodeData).lastSeen : null;
};

// Function to get a specific node
export const getNode = async (address: string) => {
  if (!address || typeof address !== "string") {
    throw new Error("Invalid address");
  }

  const rawNode = await redis.get(`node:${address}`);

  if (!rawNode) {
    throw new Error("Node not found");
  }

  const nodeResult = ComputeNodeSchema.safeParse(JSON.parse(rawNode));

  if (!nodeResult.success) {
    throw new Error("Error parsing node data");
  }

  return {
    message: "Node retrieved successfully",
    data: nodeResult.data,
  };
};

const _getNodes = async (
  filterFn: (node: ComputeNode) => boolean,
): Promise<ComputeNode[]> => {
  const keys = await redis.keys("node:*");
  const nodes = await Promise.all(
    keys.map(async (key) => {
      const rawNode = await redis.get(key);
      if (!rawNode) return null;

      const node = JSON.parse(rawNode);
      return {
        id: key.replace("node:", ""),
        ...node,
      } as ComputeNode;
    }),
  );

  return nodes.filter(
    (node): node is ComputeNode => node !== null && filterFn(node),
  );
};

export const getNodesForPool = async (
  computePoolId: number,
): Promise<ComputeNode[]> => {
  if (isNaN(computePoolId)) {
    throw new Error("Invalid compute pool ID");
  }
  return _getNodes((node) => node.computePoolId === computePoolId);
};

export const getAllNodes = async (): Promise<ComputeNode[]> => {
  return _getNodes(() => true);
};
