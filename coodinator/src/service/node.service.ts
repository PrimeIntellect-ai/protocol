import { KVStorage } from '../storage/etcd';

export type Node = {
  id: string;
  status: 'active' | 'terminated' | 'invited';
  lastHeartbeat?: Date;
};

export type NodeService = {
  getActive: () => Promise<Node[]>;
  getNode: (id: string) => Promise<Node | null>;
  heartbeat: (id: string) => Promise<void>;
  terminate: (id: string) => Promise<void>;
};

const HEARTBEAT_TTL = 30;

// Sample debug node
const DEBUG_NODE: Node = {
  id: '12345678-1234-1234-1234-123456789012',
  status: 'active',
  lastHeartbeat: new Date(),
};

export const createNodeService = (storage: KVStorage): NodeService => {
  const keyFor = (id: string, type: string) => `nodes/${id}/${type}`;

  // Initialize debug node
  storage.put(keyFor(DEBUG_NODE.id, 'status'), DEBUG_NODE.status);

  return {
    getActive: async () => {
      const nodes = await storage.getPrefix('nodes/');
      console.log(nodes);
      /*
      return nodes
        .map(n => JSON.parse(n))
        .filter(n => n.status === 'active');*/
      return [DEBUG_NODE];
    },

    getNode: async (id) => {
      const status = await storage.get(keyFor(id, 'status'));
      if (!status) {
        return null;
      }
      const heartbeat = await storage.get(keyFor(id, 'heartbeat'));
      return {
        id,
        status: status as Node['status'],
        lastHeartbeat: heartbeat ? new Date(heartbeat) : undefined,
      };
    },

    heartbeat: async (id) => {
      const nodeStatus = await storage.get(keyFor(id, 'status'));
      if (!nodeStatus) {
        throw new Error('Node does not exist');
      }
      await storage.put(
        keyFor(id, 'heartbeat'),
        new Date().toISOString(),
        HEARTBEAT_TTL
      );
    },

    terminate: (id) => storage.put(keyFor(id, 'status'), 'terminated'),
  };
};
