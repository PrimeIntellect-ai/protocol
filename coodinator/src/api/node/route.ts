import express from 'express';
import { z } from 'zod';

const router = express.Router();

// Validation schema for heartbeat request
const HeartbeatRequestSchema = z.object({
  nodeId: z.string().uuid(),
  status: z.enum(['active', 'idle', 'error']),
  metrics: z.object({
    cpuUsage: z.number().min(0).max(100),
    memoryUsage: z.number().min(0).max(100),
    timestamp: z.string().datetime()
  })
});

// Response schema
const HeartbeatResponseSchema = z.object({
  status: z.enum(['ok', 'error']),
  timestamp: z.string().datetime(),
  message: z.string().optional()
});

type HeartbeatRequest = z.infer<typeof HeartbeatRequestSchema>;
type HeartbeatResponse = z.infer<typeof HeartbeatResponseSchema>;

/**
 * POST /api/node/heartbeat
 * Endpoint for nodes to send heartbeat updates
 */
router.post('/heartbeat', async (req, res) => {
  try {
    // Validate request body
    const heartbeat = HeartbeatRequestSchema.parse(req.body);
    
    // TODO: Update node's last heartbeat timestamp in etcd
    
    const response: HeartbeatResponse = {
      status: 'ok',
      timestamp: new Date().toISOString()
    };

    res.status(200).json(response);

  } catch (error) {
    console.error('Error processing heartbeat:', error);
    
    const response: HeartbeatResponse = {
      status: 'error',
      timestamp: new Date().toISOString(),
      message: 'Failed to process heartbeat'
    };

    res.status(500).json(response);
  }
});

export default router;
