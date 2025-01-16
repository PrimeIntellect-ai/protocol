import express from 'express';
import { z } from 'zod';
import { NodeService } from '../../service/node.service';

export function createNodeRouter(nodeService: NodeService) {
  const router = express.Router();

  // Route handlers
  const handleHeartbeat = async (
    req: express.Request,
    res: express.Response
  ) => {
    const result = HeartbeatRequestSchema.safeParse(req.body);
    if (!result.success) {
      const formattedErrors = result.error.errors.map((err) => {
        if (err.code === 'invalid_type' && err.received === 'undefined') {
          return `${err.path.join('.')}: Required`;
        }
        return `${err.path.join('.')}: ${err.message}`;
      });
      res
        .status(400)
        .json({ status: 'error', message: formattedErrors.join('\n') });
      return;
    }

    await nodeService.heartbeat(result.data.nodeId);
    res.status(200).json({ status: 'ok' });
  };

  const handleGetActive = async (
    req: express.Request,
    res: express.Response
  ) => {
    const nodes = await nodeService.getActive();
    res.status(200).json(nodes);
  };

  // Routes
  router.post('/heartbeat', handleHeartbeat);
  router.get('/active', handleGetActive);

  // Schemas
  const HeartbeatRequestSchema = z.object({
    nodeId: z.string().uuid(),
    status: z.enum(['active', 'idle', 'error']),
    metrics: z
      .object({
        cpuUsage: z.number().min(0).max(100),
        memoryUsage: z.number().min(0).max(100),
        timestamp: z.string().datetime(),
      })
      .optional(),
  });

  return router;
}
