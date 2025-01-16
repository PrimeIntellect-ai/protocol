import { Etcd3 } from 'etcd3';
import express from 'express';
import helmet from 'helmet';
import compression from 'compression';
import winston from 'winston';
import { createStorage } from './storage/etcd';
import { createNodeService } from './service/node.service';
import { createNodeRouter } from './api/node/nodes';

// Configure logger
const logger = winston.createLogger({
  level: process.env.NODE_ENV === 'production' ? 'info' : 'debug',
  format: winston.format.combine(
    winston.format.timestamp(),
    winston.format.colorize(),
    winston.format.printf(({ level, message, timestamp }) => {
      return `${timestamp} ${level}: ${message}`;
    })
  ),
  transports: [
    new winston.transports.Console(),
    new winston.transports.File({
      filename: 'error.log',
      level: 'error',
      format: winston.format.json(),
    }),
    new winston.transports.File({
      filename: 'combined.log',
      format: winston.format.json(),
    }),
  ],
  exceptionHandlers: [
    new winston.transports.File({ filename: 'exceptions.log' }),
  ],
  exitOnError: false,
});

async function main() {
  const app = express();
  const port = process.env.PORT || 3000;

  const client = new Etcd3({
    hosts: process.env.ETCD_HOSTS?.split(',') || ['http://etcd:2379'],
  });

  const storage = createStorage(client);
  const nodeService = createNodeService(storage);

  try {
    // Security middleware
    app.use(helmet());
    app.disable('x-powered-by');

    // Performance middleware
    app.use(compression()); // Enable gzip compression

    // Request parsing
    app.use(express.json({ limit: '10mb' }));
    app.use(express.urlencoded({ extended: true }));

    // Routes
    app.use('/api/nodes', createNodeRouter(nodeService));

    // 404 handler
    app.use((req, res) => {
      res.status(404).json({ error: 'Not found' });
    });

    // Error handling middleware
    app.use(
      (
        err: Error,
        req: express.Request,
        res: express.Response,
        next: express.NextFunction
      ) => {
        logger.error('Unhandled error:', { error: err });
        res.status(500).json({ error: 'Internal server error' });
      }
    );

    // Start server
    const server = app.listen(port, () => {
      logger.info(
        `Server listening on port ${port} in ${process.env.NODE_ENV || 'development'} mode`
      );
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
        logger.error(
          'Could not close connections in time, forcefully shutting down'
        );
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
