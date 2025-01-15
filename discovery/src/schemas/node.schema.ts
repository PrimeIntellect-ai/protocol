import { z } from 'zod'

export const NodeSchema = z.object({
  id: z.string().optional(),
  ipAddress: z.string().ip(),
  port: z.number().int().min(1).max(65535),
  computePoolId: z.number().int().min(0).optional(),
  lastSeen: z.number().int().min(0).optional(),
})

export type Node = z.infer<typeof NodeSchema>
