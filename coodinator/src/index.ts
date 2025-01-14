import { Etcd3 } from 'etcd3';
import express from 'express';
import helmet from 'helmet';
import compression from 'compression';
import router from './api';
import winston from 'winston';

// Configure logger
const logger = winston.createLogger({
  level: process.env.NODE_ENV === 'production' ? 'info' : 'debug',
  format: winston.format.combine(
    winston.format.timestamp(),
    winston.format.json()
  ),
  transports: [
    new winston.transports.Console(),
    new winston.transports.File({ filename: 'error.log', level: 'error' }),
    new winston.transports.File({ filename: 'combined.log' })
  ]
});

async function main() {
  const app = express();
  const port = process.env.PORT || 3000;

  const client = new Etcd3({
    hosts: process.env.ETCD_HOSTS?.split(',') || ['http://etcd:2379']
  });

  try {
    // Test etcd connection
    await client.put('test-key').value('test');
    const value = await client.get('test-key').string();
    logger.info('Retrieved from etcd:', { value });

    // Security middleware
    app.use(helmet());
    app.disable('x-powered-by');

    // Performance middleware
    app.use(compression()); // Enable gzip compression
    
    // Request parsing
    app.use(express.json({ limit: '10mb' }));
    app.use(express.urlencoded({ extended: true }));

    // Routes
    app.use('/api', router);

    // 404 handler
    app.use((req, res) => {
      res.status(404).json({ error: 'Not found' });
    });

    // Error handling middleware
    app.use((err: Error, req: express.Request, res: express.Response, next: express.NextFunction) => {
      logger.error('Unhandled error:', { error: err.stack });
      res.status(500).json({ error: 'Internal server error' });
    });

    // Start server
    const server = app.listen(port, () => {
      logger.info(`Server listening on port ${port} in ${process.env.NODE_ENV || 'development'} mode`);
    });

    // Graceful shutdown
    const shutdown = async () => {
      logger.info('Shutdown signal received');
      
      server.close(() => {
        logger.info('HTTP server closed');
        client.close();
        process.exit(0);
      });

      // Force close after 10s
      setTimeout(() => {
        logger.error('Could not close connections in time, forcefully shutting down');
        process.exit(1);
      }, 10000);
    };

    process.on('SIGTERM', shutdown);
    process.on('SIGINT', shutdown);

  } catch (error) {
    logger.error('Startup error:', { error });
    process.exit(1);
  }
}

// Handle uncaught errors
process.on('uncaughtException', (error) => {
  logger.error('Uncaught exception:', { error });
  process.exit(1);
});

process.on('unhandledRejection', (error) => {
  logger.error('Unhandled rejection:', { error });
  process.exit(1);
});

main().catch((error) => {
  logger.error('Unhandled rejection:', { error });
  process.exit(1);
});