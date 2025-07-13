#!/usr/bin/env python3
"""
Integration test for the authentication flow between two nodes.

This test demonstrates:
1. Two nodes starting up with their own wallets
2. First message triggers authentication
3. Addresses are extracted from signatures
4. Subsequent messages are sent directly
"""

import asyncio
import pytest
import time
from primeprotocol import WorkerClient
import logging

# Set up logging
logging.basicConfig(level=logging.DEBUG)


@pytest.fixture
def setup_test_environment():
    """Set up test environment with Anvil keys"""
    # Anvil test keys
    node_a_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
    node_b_key = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
    provider_key = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"
    
    return {
        "rpc_url": "http://localhost:8545",
        "pool_id": 0,
        "node_a": {"node_key": node_a_key, "provider_key": provider_key, "port": 9001},
        "node_b": {"node_key": node_b_key, "provider_key": provider_key, "port": 9002},
    }


@pytest.mark.asyncio
async def test_authentication_flow(setup_test_environment):
    """Test the full authentication flow between two nodes"""
    env = setup_test_environment
    
    # Create two worker clients
    client_a = WorkerClient(
        compute_pool_id=env["pool_id"],
        rpc_url=env["rpc_url"],
        private_key_provider=env["node_a"]["provider_key"],
        private_key_node=env["node_a"]["node_key"],
        p2p_port=env["node_a"]["port"]
    )
    
    client_b = WorkerClient(
        compute_pool_id=env["pool_id"],
        rpc_url=env["rpc_url"],
        private_key_provider=env["node_b"]["provider_key"],
        private_key_node=env["node_b"]["node_key"],
        p2p_port=env["node_b"]["port"]
    )
    
    try:
        # Start both clients
        logging.info("Starting client A...")
        client_a.start()
        peer_a_id = client_a.get_own_peer_id()
        logging.info(f"Client A started with peer ID: {peer_a_id}")
        
        logging.info("Starting client B...")
        client_b.start()
        peer_b_id = client_b.get_own_peer_id()
        logging.info(f"Client B started with peer ID: {peer_b_id}")
        
        # Give nodes time to start
        await asyncio.sleep(2)
        
        # Test 1: First message from A to B (triggers authentication)
        logging.info("Test 1: Sending first message from A to B...")
        client_a.send_message(
            peer_id=peer_b_id,
            data=b"Hello from A! This is the first message.",
            multiaddrs=[f"/ip4/127.0.0.1/tcp/{env['node_b']['port']}"]
        )
        
        # Check if B receives the message
        message = None
        for _ in range(50):  # Poll for up to 5 seconds
            message = client_b.get_next_message()
            if message:
                break
            await asyncio.sleep(0.1)
        
        assert message is not None, "Client B did not receive message"
        assert message["message"]["type"] == "general"
        assert bytes(message["message"]["data"]) == b"Hello from A! This is the first message."
        assert message["sender_address"] is not None  # Should have sender's address
        logging.info(f"Client B received message with sender address: {message['sender_address']}")
        
        # Test 2: Second message from A to B (should be direct, no auth)
        logging.info("Test 2: Sending second message from A to B...")
        client_a.send_message(
            peer_id=peer_b_id,
            data=b"Second message - should be direct",
            multiaddrs=[f"/ip4/127.0.0.1/tcp/{env['node_b']['port']}"]
        )
        
        # Should receive quickly since already authenticated
        message = None
        for _ in range(20):  # Should be faster
            message = client_b.get_next_message()
            if message:
                break
            await asyncio.sleep(0.1)
        
        assert message is not None, "Client B did not receive second message"
        assert bytes(message["message"]["data"]) == b"Second message - should be direct"
        
        # Test 3: Message from B to A (first message in this direction)
        logging.info("Test 3: Sending message from B to A...")
        client_b.send_message(
            peer_id=peer_a_id,
            data=b"Hello from B!",
            multiaddrs=[f"/ip4/127.0.0.1/tcp/{env['node_a']['port']}"]
        )
        
        # Check if A receives the message
        message = None
        for _ in range(50):
            message = client_a.get_next_message()
            if message:
                break
            await asyncio.sleep(0.1)
        
        assert message is not None, "Client A did not receive message"
        assert bytes(message["message"]["data"]) == b"Hello from B!"
        assert message["sender_address"] is not None
        logging.info(f"Client A received message with sender address: {message['sender_address']}")
        
        logging.info("All tests passed! Authentication flow working correctly.")
        
    finally:
        # Clean up
        client_a.stop()
        client_b.stop()


if __name__ == "__main__":
    # Run the test
    asyncio.run(test_authentication_flow({})) 