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
        capacity: 100,
        computePoolId: 0,
      }

      // Create signature
      const message = `/nodes/${wallet.address}` + JSON.stringify(nodeData, Object.keys(nodeData).sort())
      const signature = await wallet.signMessage(message)

      // Prepare the complete request body
      const requestBody = {
        ...nodeData,
        signature,
      }

      const response = await request(app)
        .put(`/nodes/${wallet.address}`)
        .set('x-eth-address', wallet.address) // Set the verified address in the headers
        .send(requestBody)

      expect(response.status).toBe(200)
      expect(response.body.success).toBe(true)
      console.log(response.body.data)
      console.log(wallet.address)
      expect(response.body.data).toMatchObject({
        address: wallet.address,
        capacity: nodeData.capacity,
        computePoolId: 0,
        ipAddress: nodeData.ipAddress,
        port: nodeData.port,
      })
    })

    it('should not register a node with invalid address', async () => {
      // Create a test wallet
      const wallet = ethers.Wallet.createRandom()
      const wrongAddress = ethers.Wallet.createRandom().address // Generate a different address
      
      // Prepare node data
      const nodeData = {
        ipAddress: '192.168.1.100',
        port: 8545,
        capacity: 100
      }

      // Create signature
      const message = `/nodes/${wallet.address}` + JSON.stringify(nodeData, Object.keys(nodeData).sort())
      const signature = await wallet.signMessage(message)

      // Prepare the complete request body with the wrong address
      const requestBody = {
        ...nodeData,
        signature,
      }

      const response = await request(app)
        .put(`/nodes/${wrongAddress}`)
        .set('x-eth-address', wallet.address) // Set the verified address in the headers
        .send(requestBody)

      expect(response.status).toBe(401) // Expect unauthorized status
      expect(response.body.success).toBe(false)
      expect(response.body.message).toBe('Invalid signature') // Expect specific error message
    })
  })
})