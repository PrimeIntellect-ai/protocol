import { z } from 'zod'

export const ComputeNodeSchema = z.object({
  id: z.string().optional(),
  ipAddress: z.string().ip(),
  port: z.number().int().min(1).max(65535),
  computePoolId: z.number().int().min(0).optional(),
  lastSeen: z.number().int().min(0).optional(),
})

export type ComputeNode = z.infer<typeof ComputeNodeSchema>
