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


def handle_pool_owner_message(message: Dict[str, Any]) -> None:
    """Handle messages from pool owner"""
    logging.info(f"Received message from pool owner: {message}")
    
    if message.get("type") == "inference_request":
        prompt = message.get("prompt", "")
        # Simulate processing the inference request
        response = f"Processed: {prompt}"
        
        logging.info(f"Processing inference request: {prompt}")
        logging.info(f"Generated response: {response}")
        
        # In a real implementation, you would send the response back
        # client.send_response({"type": "inference_response", "result": response})
    else:
        logging.info("Sending PONG response")
        # client.send_response("PONG")


def handle_validator_message(message: Dict[str, Any]) -> None:
    """Handle messages from validator"""
    logging.info(f"Received message from validator: {message}")
    
    if message.get("type") == "inference_request":
        prompt = message.get("prompt", "")
        # Simulate processing the inference request
        response = f"Validated: {prompt}"
        
        logging.info(f"Processing validation request: {prompt}")
        logging.info(f"Generated response: {response}")
        
        # In a real implementation, you would send the response back
        # client.send_response({"type": "inference_response", "result": response})


def check_for_messages(client: WorkerClient) -> None:
    """Check for new messages from pool owner and validator"""
    try:
        # Check for pool owner messages
        pool_owner_message = client.get_pool_owner_message()
        if pool_owner_message:
            handle_pool_owner_message(pool_owner_message)
        
        # Check for validator messages
        validator_message = client.get_validator_message()
        if validator_message:
            handle_validator_message(validator_message)
            
    except Exception as e:
        logging.error(f"Error checking for messages: {e}")


def main():
    rpc_url = os.getenv("RPC_URL", "http://localhost:8545")
    pool_id = os.getenv("POOL_ID", 0)
    private_key_provider = os.getenv("PRIVATE_KEY_PROVIDER", None)
    private_key_node = os.getenv("PRIVATE_KEY_NODE", None)

    logging.info(f"Connecting to: {rpc_url}")
    client = WorkerClient(pool_id, rpc_url, private_key_provider, private_key_node)
    
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
        logging.info("Setup completed. Starting message polling loop...")
        print("Worker client started. Polling for messages. Press Ctrl+C to stop.")
        
        # Message polling loop
        while True:
            try:
                check_for_messages(client)
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