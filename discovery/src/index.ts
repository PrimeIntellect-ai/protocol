import express, { Request, Response, NextFunction } from 'express'
import helmet from 'helmet'
import morgan from 'morgan'
import { redis } from './config/redis'
import { ApiResponse } from './schemas/apiResponse.schema'
import nodesRouter from './routes/nodes.route'
import swaggerUi from 'swagger-ui-express'
import swaggerJSDoc from 'swagger-jsdoc'

const swaggerDefinition = {
  openapi: '3.0.0',
  info: {
    title: 'Discovery Service API',
    version: '1.0.0',
    description: 'API documentation for the Discovery Service',
  },
  servers: [
    {
      url: `http://localhost:${process.env.PORT || 8089}`,
    },
  ],
}

const options = {
  swaggerDefinition,
  apis: ['./src/routes/*.ts'],
}

const swaggerSpec = swaggerJSDoc(options)

const app = express()
app.use(express.json())
app.use(helmet())
app.use(morgan('combined'))
app.use('/api', nodesRouter)

// Disable Swagger documentation in production environment
if (process.env.NODE_ENV !== 'production') {
  app.use('/docs', swaggerUi.serve, swaggerUi.setup(swaggerSpec))
  console.log('API documentation available at /docs')
}

app.use(
  (
    error: Error,
    req: Request,
    res: Response<ApiResponse<never>>,
    next: NextFunction
  ) => {
    console.error('Unhandled error:', error)
    res.status(500).json({
      success: false,
      message: 'Internal server error',
      error: error.message,
    })
  }
)

const port = process.env.PORT || 8089
if (require.main === module) {
  app.listen(port, () => console.log(`Discovery service running on ${port}`))
}

export { app, redis }
