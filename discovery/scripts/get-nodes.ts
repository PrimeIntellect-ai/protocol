import dotenv from 'dotenv';

dotenv.config();

import { ethers } from 'ethers';

const BASE_URL = 'http://localhost:8089';

async function getPoolNodes(wallet: ethers.Wallet, poolId: number) {
    const url = `${BASE_URL}/nodes/pool/${poolId}`;
    const message = `/nodes/pool/${poolId}`; // Updated to exclude BASE_URL
    const signature = await wallet.signMessage(message);
    console.log('Signature for getting nodes:', signature);

    // Call the API to get nodes
    const response = await fetch(url, {
        headers: {
            'Content-Type': 'application/json',
            'x-eth-address': wallet.address,
            'x-signature': signature
        },
    });
    const data = await response.json();
    console.log('Nodes:', data);
}

async function getValidatorNodes(testAddress?: string) {
    const wallet = new ethers.Wallet(process.env.PRIVATE_KEY_VALIDATOR!);
    const validatorAddress = testAddress || wallet.address;

    const url = `${BASE_URL}/nodes/validator`;
    const message = `/nodes/validator`; // Updated to exclude BASE_URL
    const signature = await wallet.signMessage(message);

    // Call the API to get nodes for the specific validator
    const response = await fetch(url, {
        headers: {
            'Content-Type': 'application/json',
            'x-eth-address': validatorAddress,
            'x-signature': signature,
        },
    });
    const data = await response.json();
    console.log('Validator Nodes:', data);
}

async function addNode(wallet: ethers.Wallet, nodeData: { ipAddress: string; port: number; capacity: number; computePoolId: number; }) {
    const addNodeUrl = `${BASE_URL}/nodes/${wallet.address}`; // Include the full URL
    const addNodeMessage = `/nodes/${wallet.address}` + JSON.stringify(nodeData, Object.keys(nodeData).sort()); // Updated to exclude BASE_URL
    const addNodeSignature = await wallet.signMessage(addNodeMessage);
    console.log('Add Node Signature:', addNodeSignature);

    const addNodeResponse = await fetch(addNodeUrl, {
        method: 'PUT',
        headers: {
            'Content-Type': 'application/json',
            'x-eth-address': wallet.address,
            'x-signature': addNodeSignature
        },
        body: JSON.stringify(nodeData),
    });

    if (!addNodeResponse.ok) { // Check if the response is not ok
        throw new Error(`Failed to add node: ${addNodeResponse.statusText}`);
    }

    const addNodeData = await addNodeResponse.json();
    console.log('Add Node Response:', addNodeData);
}

async function main() {
    const wallet = new ethers.Wallet(process.env.POOL_OWNER_PRIVATE_KEY!);

    const poolId = 0;
    // Add a new node
    const newNode = {
        ipAddress: '192.168.1.100',
        port: 8545,
        capacity: 100,
        computePoolId: poolId,
    };
    //await addNode(wallet, newNode);

    await getPoolNodes(wallet, poolId);

    // Example usage of getting validator nodes
    await getValidatorNodes();
    await getValidatorNodes(wallet.address);
}

main().catch(console.error);
