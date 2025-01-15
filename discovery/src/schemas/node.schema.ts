import { z } from "zod";

export const ComputeNodeSchema = z.object({
  id: z.string().optional(),
  ipAddress: z.string().ip(),
  port: z.number().int().min(1).max(65535),
  computePoolId: z.number().int().min(0).optional(),
  lastSeen: z.number().int().min(0).optional(),

  // Specifications for compute resources
  computeSpecs: z
    .object({
      // GPU specifications
      gpu: z
        .object({
          count: z.number().int().min(0).optional(), // Number of GPUs available
          model: z.string().optional(), // Model of the GPU
          memoryMB: z.number().int().min(0).optional(), // Memory of the GPU in MB
        })
        .optional(),

      // CPU specifications
      cpu: z
        .object({
          cores: z.number().int().min(1).optional(), // Number of CPU cores
          model: z.string().optional(), // Model of the CPU
        })
        .optional(),

      // Memory and storage specifications
      ramMB: z.number().int().min(0).optional(), // RAM in MB
      storageGB: z.number().int().min(0).optional(), // Storage in GB
    })
    .optional(),
});

export type ComputeNode = z.infer<typeof ComputeNodeSchema>;
