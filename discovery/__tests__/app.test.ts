import request from 'supertest'
import { ethers } from 'ethers'
import { app } from '../src/index'
import { describe, it, expect } from '@jest/globals';

describe('Node API', () => {
  describe('PUT /nodes/:nodeId', () => {
    it('should register a node with valid signature', async () => {
      // Create a test wallet
      const wallet = ethers.Wallet.createRandom()
      
      // Prepare node data
      const nodeData = {
        ipAddress: '192.168.1.100',
        port: 8545,
        capacity: 100
      }

      // Create signature
      const message = wallet.address + JSON.stringify(nodeData, Object.keys(nodeData).sort())
      const signature = await wallet.signMessage(message)

      // Prepare the complete request body
      const requestBody = {
        ...nodeData,
        signature,
      }

      const response = await request(app)
        .put(`/nodes/${wallet.address}`)
        .send(requestBody)

      expect(response.status).toBe(200)
      expect(response.body.success).toBe(true)
      expect(response.body.data).toMatchObject({
        nodeId: wallet.address,
        ipAddress: nodeData.ipAddress,
        port: nodeData.port,
        capacity: nodeData.capacity
      })
    })
  })
})