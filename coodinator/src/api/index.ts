import express from 'express';
import nodeRouter from './node/route';

const router = express.Router();

router.use('/node', nodeRouter);

export default router;
