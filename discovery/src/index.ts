import express, { Request, Response, NextFunction } from 'express'
import helmet from 'helmet'
import morgan from 'morgan'
import { redis } from './config/redis'
import { ApiResponse } from './schemas/apiResponse.schema'
import nodesRouter from './routes/nodes.route' // Importing the nodes route

const app = express()
app.use(express.json())
app.use(helmet())
app.use(morgan('combined'))
app.use('/api', nodesRouter)

// Error handling middleware
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
