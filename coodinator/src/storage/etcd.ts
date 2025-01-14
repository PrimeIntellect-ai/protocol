import { Etcd3, Lease } from 'etcd3';

export type KVStorage = {
  get: (key: string) => Promise<string | null>;
  getPrefix: (prefix: string) => Promise<string[]>;
  put: (key: string, value: string, ttl?: number) => Promise<void>;
  delete: (key: string) => Promise<void>;
};

export const createStorage = (client: Etcd3): KVStorage => ({
  get: (key) => client.get(key).string(),
  put: async (key: string, value: string, ttl?: number) => {
    let request = client.put(key).value(value);
    if (ttl) {
      const lease = client.lease(ttl);
      const leaseId = await lease.grant();
      request = request.lease(leaseId);
    }
    await request;
  },
  getPrefix: async (prefix) => {
    const values = await client.getAll().prefix(prefix).strings();
    return Object.values(values);
  },
  delete: async (key) => {
    await client.delete().key(key).exec();
  },
});
