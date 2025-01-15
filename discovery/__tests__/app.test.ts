import request from 'supertest'
import { ethers } from 'ethers'
import { app } from '../src/index'
import { describe, it, expect } from '@jest/globals';

describe('Node API', () => {
  describe('PUT /api/nodes/:nodeId', () => {
    it('should register a node with valid signature', async () => {
      const wallet = ethers.Wallet.createRandom()
      const nodeData = {
        ipAddress: '192.168.1.100',
        port: 8545,
        computePoolId: 0,
      }
      const message = `/api/nodes/${wallet.address}` + JSON.stringify(nodeData, Object.keys(nodeData).sort())
      const signature = await wallet.signMessage(message)

      const response = await request(app)
        .put(`/api/nodes/${wallet.address}`)
        .set('x-address', wallet.address)
        .set('x-signature', signature)
        .send(nodeData)

      expect(response.status).toBe(200)
      expect(response.body.success).toBe(true)
      expect(response.body.data).toMatchObject({
        address: wallet.address,
        computePoolId: 0,
        ipAddress: nodeData.ipAddress,
        port: nodeData.port,
      })
    })

    it('should not register a node with invalid address', async () => {
      const wallet = ethers.Wallet.createRandom()
      const wrongAddress = ethers.Wallet.createRandom().address
      const nodeData = {
        ipAddress: '192.168.1.100',
        port: 8545,
      }
      const message = `/api/nodes/${wallet.address}` + JSON.stringify(nodeData, Object.keys(nodeData).sort())
      const signature = await wallet.signMessage(message)

      const response = await request(app)
        .put(`/api/nodes/${wrongAddress}`)
        .set('x-address', wallet.address)
        .set('x-signature', signature)
        .send(nodeData)

      expect(response.status).toBe(401)
      expect(response.body.success).toBe(false)
      expect(response.body.message).toBe('Invalid signature')
    })

    it('should not allow updating a node too quickly', async () => {
      const wallet = ethers.Wallet.createRandom()
      const nodeData = {
        ipAddress: '192.168.1.100',
        port: 8545,
        computePoolId: 0,
      }
      const message = `/api/nodes/${wallet.address}` + JSON.stringify(nodeData, Object.keys(nodeData).sort())
      const signature = await wallet.signMessage(message)

      // First update
      await request(app)
        .put(`/api/nodes/${wallet.address}`)
        .set('x-address', wallet.address)
        .set('x-signature', signature)
        .send(nodeData)

      // Attempt to update again immediately
      const response = await request(app)
        .put(`/api/nodes/${wallet.address}`)
        .set('x-address', wallet.address)
        .set('x-signature', signature)
        .send(nodeData)

      expect(response.status).toBe(429)
      expect(response.body.success).toBe(false)
      expect(response.body.message).toBe('Please wait 5 minutes between updates')
    })
  })
})