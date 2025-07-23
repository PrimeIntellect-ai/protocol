#!/usr/bin/env python3
"""Example usage of the Prime Protocol Python client."""

import asyncio
import logging
import os
import signal
import sys
import time
from typing import Dict, Any, Optional
from primeprotocol import WorkerClient

FORMAT = '%(levelname)s %(name)s %(asctime)-15s %(filename)s:%(lineno)d %(message)s'
logging.basicConfig(format=FORMAT)
logging.getLogger().setLevel(logging.INFO)


def main():
    rpc_url = os.getenv("RPC_URL", "http://localhost:8545")
    pool_id = os.getenv("POOL_ID", 0)
    private_key_provider = os.getenv("PRIVATE_KEY_PROVIDER", None)
    private_key_node = os.getenv("PRIVATE_KEY_NODE", None)

    logging.info(f"Connecting to: {rpc_url}")

    port = int(os.getenv("PORT", 8003))
    client = WorkerClient(pool_id, rpc_url, private_key_provider, private_key_node, port)
    
    def signal_handler(sig, frame):
        logging.info("Received interrupt signal, shutting down gracefully...")
        try:
            client.stop()
            logging.info("Client stopped successfully")
        except Exception as e:
            logging.error(f"Error during shutdown: {e}")
        sys.exit(0)
    
    # Register signal handler for Ctrl+C before starting client
    signal.signal(signal.SIGINT, signal_handler)
    signal.signal(signal.SIGTERM, signal_handler)
    
    try:
        logging.info("Starting client... (Press Ctrl+C to interrupt)")
        client.start()

        client.upload_to_discovery("127.0.0.1", None)

        my_peer_id = client.get_own_peer_id()
        logging.info(f"My Peer ID: {my_peer_id}")

        time.sleep(5)

        # Note: To send messages to other peers manually, you can use:
        # peer_id = "12D3KooWELi4p1oR3QBSYiq1rvPpyjbkiQVhQJqCobBBUS7C6JrX"
        # peer_multi_addr = "/ip4/127.0.0.1/tcp/8002"
        # client.send_message(peer_id, b"Hello, world!", [peer_multi_addr])

        logging.info("Setup completed. Starting message polling loop...")
        print("Worker client started. Polling for orchestrator/validator messages. Press Ctrl+C to stop.")
        
        # Message polling loop - listening for orchestrator and validator messages
        while True:
            try:
                message = client.get_next_message()
                if message:
                    msg_data = message.get('message', {})
                    if msg_data.get('type') == 'general':
                        data = bytes(msg_data.get('data', []))
                        print(f"Message from {message['peer_id']}: {data}")
                    else:
                        print(f"Message from {message['peer_id']}: type={msg_data.get('type')}")
                        
                time.sleep(0.1)  # Small delay to prevent busy waiting
            except KeyboardInterrupt:
                # Handle Ctrl+C during message polling
                logging.info("Keyboard interrupt received during polling")
                signal_handler(signal.SIGINT, None)
                break
            
    except KeyboardInterrupt:
        # Handle Ctrl+C during client startup
        logging.info("Keyboard interrupt received during startup")
        signal_handler(signal.SIGINT, None)
    except Exception as e:
        logging.error(f"Unexpected error: {e}")
        try:
            client.stop()
        except:
            pass
        sys.exit(1)

if __name__ == "__main__":
    main() 