import request from 'supertest'
import { ethers } from 'ethers'
import { app } from '../src/index'
import { describe, it, expect } from '@jest/globals';

describe('Node API', () => {
  describe('PUT /nodes/:nodeId', () => {
    it('should register a node with valid signature', async () => {
      const wallet = ethers.Wallet.createRandom()
      const nodeData = {
        ipAddress: '192.168.1.100',
        port: 8545,
        capacity: 100,
        computePoolId: 0,
      }
      const message = `/nodes/${wallet.address}` + JSON.stringify(nodeData, Object.keys(nodeData).sort())
      const signature = await wallet.signMessage(message)

      const response = await request(app)
        .put(`/nodes/${wallet.address}`)
        .set('x-eth-address', wallet.address)
        .set('x-signature', signature)
        .send(nodeData)

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
      const wallet = ethers.Wallet.createRandom()
      const wrongAddress = ethers.Wallet.createRandom().address
      const nodeData = {
        ipAddress: '192.168.1.100',
        port: 8545,
        capacity: 100
      }
      const message = `/nodes/${wallet.address}` + JSON.stringify(nodeData, Object.keys(nodeData).sort())
      const signature = await wallet.signMessage(message)

      const response = await request(app)
        .put(`/nodes/${wrongAddress}`)
        .set('x-eth-address', wallet.address)
        .set('x-signature', signature)
        .send(nodeData)

      expect(response.status).toBe(401)
      expect(response.body.success).toBe(false)
      expect(response.body.message).toBe('Invalid signature')
    })
  })
})