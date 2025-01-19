import express, { NextFunction, Request, Response } from "express";
import {
  verifyPoolOwner,
  verifyPrimeValidator,
  verifySignature,
} from "../middleware/auth";
import { ApiResponse } from "../schemas/apiResponse.schema";
import { ComputeNode } from "../schemas/node.schema";
import {
  getAllNodes,
  getNode,
  getNodesForPool,
  registerNode,
} from "../service/nodes.service";

const router = express.Router();

/**
 * @swagger
 * /nodes/{address}:
 *   put:
 *     summary: Register a new node
 *     parameters:
 *       - in: path
 *         name: address
 *         required: true
 *         description: The address of the node owner
 *         schema:
 *           type: string
 *     requestBody:
 *       required: true
 *       content:
 *         application/json:
 *           schema:
 *             $ref: '#/components/schemas/ComputeNode'
 *     responses:
 *       200:
 *         description: Node registered successfully
 *         content:
 *           application/json:
 *             schema:
 *               type: object
 *               properties:
 *                 success:
 *                   type: boolean
 *                 message:
 *                   type: string
 *                 data:
 *                   type: object
 *                   properties:
 *                     address:
 *                       type: string
 *                     ipAddress:
 *                       type: string
 *                     port:
 *                       type: integer
 *                     computePoolId:
 *                       type: integer
 *                     lastSeen:
 *                       type: integer
 *       429:
 *         description: Too many requests
 *         content:
 *           application/json:
 *             schema:
 *               type: object
 *               properties:
 *                 success:
 *                   type: boolean
 *                 message:
 *                   type: string
 */
router.put<{ address: string }, ApiResponse<{ address: string }>>(
  "/nodes/:address",
  verifySignature,
  async (
    req: Request,
    res: Response<ApiResponse<{ address: string }>>,
    next: NextFunction,
  ) => {
    try {
      const address = req.params.address;
      const nodeData = req.body;
      const response = await registerNode(address, nodeData);

      res.status(200).json({
        success: true,
        message: response.message,
        data: {
          address: address,
          ...nodeData,
        },
      });
    } catch (error: unknown) {
      if (
        error instanceof Error &&
        error.message === "Please wait 5 minutes between updates"
      ) {
        res.status(429).json({
          success: false,
          message: error.message,
        });
      } else {
        next(error);
      }
    }
  },
);

/**
 * @swagger
 * /nodes/single/{address}:
 *   get:
 *     summary: Get a specific node
 *     parameters:
 *       - in: path
 *         name: address
 *         required: true
 *         description: The address of the node
 *         schema:
 *           type: string
 *     responses:
 *       200:
 *         description: Node retrieved successfully
 *         content:
 *           application/json:
 *             schema:
 *               type: object
 *               properties:
 *                 success:
 *                   type: boolean
 *                 message:
 *                   type: string
 *                 data:
 *                   $ref: '#/components/schemas/ComputeNode'
 */
router.get<{ address: string }, ApiResponse<ComputeNode>>(
  "/nodes/single/:address",
  verifySignature,
  async (
    req: Request,
    res: Response<ApiResponse<ComputeNode>>,
    next: NextFunction,
  ) => {
    try {
      const nodeResult = await getNode(req.params.address);

      res.json({
        success: true,
        message: "Node retrieved successfully",
        data: nodeResult.data,
      });
    } catch (error) {
      next(error);
    }
  },
);

/**
 * @swagger
 * /nodes/validator:
 *   get:
 *     summary: List all nodes for validator
 *     responses:
 *       200:
 *         description: Nodes retrieved successfully for validator
 *         content:
 *           application/json:
 *             schema:
 *               type: object
 *               properties:
 *                 success:
 *                   type: boolean
 *                 message:
 *                   type: string
 *                 data:
 *                   type: array
 *                   items:
 *                     $ref: '#/components/schemas/ComputeNode'
 */
router.get<{}, ApiResponse<ComputeNode[]>>(
  "/nodes/validator",
  verifySignature,
  verifyPrimeValidator,
  async (
    req: Request,
    res: Response<ApiResponse<ComputeNode[]>>,
    next: NextFunction,
  ) => {
    try {
      const nodes = await getAllNodes();

      if (nodes.length === 0) {
        res.status(200).json({
          success: true,
          message: "No nodes found for validator",
          data: [],
        });
        return;
      }

      res.status(200).json({
        success: true,
        message: "Nodes retrieved successfully for validator",
        data: nodes,
      });
    } catch (error) {
      next(error);
    }
  },
);

// TODO: Missing auth
router.get<{}, ApiResponse<ComputeNode[]>>(
  "/nodes/platform",
  async (
    req: Request,
    res: Response<ApiResponse<ComputeNode[]>>,
    next: NextFunction,
  ) => {
    try {
      const nodes = await getAllNodes();

      res.status(200).json({
        success: true,
        message: "Nodes retrieved successfully for platform",
        data: nodes,
      });
    } catch (error) {
      next(error);
    }
  },
);

/**
 * @swagger
 * /nodes/pool/{computePoolId}:
 *   get:
 *     summary: List all nodes for a specific compute pool
 *     parameters:
 *       - in: path
 *         name: computePoolId
 *         required: true
 *         description: The ID of the compute pool
 *         schema:
 *           type: integer
 *     responses:
 *       200:
 *         description: Nodes retrieved successfully for compute pool
 *         content:
 *           application/json:
 *             schema:
 *               type: object
 *               properties:
 *                 success:
 *                   type: boolean
 *                 message:
 *                   type: string
 *                 data:
 *                   type: array
 *                   items:
 *                     $ref: '#/components/schemas/ComputeNode'
 */
router.get<{ computePoolId: string }, ApiResponse<ComputeNode[]>>(
  "/nodes/pool/:computePoolId",
  verifySignature,
  verifyPoolOwner,
  async (
    req: Request<{ computePoolId: string }>,
    res: Response<ApiResponse<ComputeNode[]>>,
    next: NextFunction,
  ) => {
    try {
      const computePoolId = parseInt(req.params.computePoolId);
      const nodes = await getNodesForPool(computePoolId);

      res.json({
        success: true,
        message: "Nodes retrieved successfully for compute pool",
        data: nodes,
      } as ApiResponse<ComputeNode[]>);
    } catch (error) {
      next(error);
    }
  },
);

export default router;
