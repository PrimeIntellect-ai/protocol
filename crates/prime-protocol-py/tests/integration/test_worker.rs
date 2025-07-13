#[cfg(test)]
mod worker_integration_tests {
    use prime_protocol_py::worker::{Message, WorkerClientCore};
    use test_log::test;
    use tokio_util::sync::CancellationToken;

    // Note: These tests require a running local blockchain with deployed contracts
    // Run with: cargo test --test integration -- --ignored

    #[test(tokio::test)]
    #[ignore = "requires local blockchain setup"]
    async fn test_worker_full_lifecycle() {
        // Standard Anvil test keys
        let node_key = "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6";
        let provider_key = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";
        let cancellation_token = CancellationToken::new();

        let mut worker = WorkerClientCore::new(
            0, // compute_pool_id
            "http://localhost:8545".to_string(),
            Some(provider_key.to_string()),
            Some(node_key.to_string()),
            Some(true), // auto_accept_transactions
            Some(5),    // funding_retry_count
            cancellation_token.clone(),
            8000, // p2p_port
        )
        .expect("Failed to create worker");

        // Start the worker
        worker
            .start_async()
            .await
            .expect("Failed to start worker");

        // Verify peer ID was assigned
        let peer_id = worker.get_peer_id();
        assert!(peer_id.is_some());
        println!("Worker started with peer ID: {:?}", peer_id);

        // Test message sending to self
        let test_message = Message {
            data: b"Hello, self!".to_vec(),
            peer_id: peer_id.unwrap().to_string(),
            multiaddrs: vec![],
        };

        worker
            .send_message(test_message)
            .await
            .expect("Failed to send message");

        // Give some time for the message to be processed
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Try to receive the message
        let received = worker.get_next_message().await;
        assert!(received.is_some());

        let msg = received.unwrap();
        assert_eq!(msg.data, b"Hello, self!");

        // Clean shutdown
        worker
            .stop_async()
            .await
            .expect("Failed to stop worker");
    }

    #[test(tokio::test)]
    #[ignore = "requires local blockchain setup"]
    async fn test_multiple_workers_communication() {
        let node_key1 = "0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6";
        let provider_key1 = "0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a";
        
        let node_key2 = "0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a";
        let provider_key2 = "0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba";

        let cancel_token1 = CancellationToken::new();
        let cancel_token2 = CancellationToken::new();

        // Create two workers
        let mut worker1 = WorkerClientCore::new(
            0,
            "http://localhost:8545".to_string(),
            Some(provider_key1.to_string()),
            Some(node_key1.to_string()),
            Some(true),
            Some(5),
            cancel_token1.clone(),
            8001,
        )
        .expect("Failed to create worker1");

        let mut worker2 = WorkerClientCore::new(
            0,
            "http://localhost:8545".to_string(),
            Some(provider_key2.to_string()),
            Some(node_key2.to_string()),
            Some(true),
            Some(5),
            cancel_token2.clone(),
            8002,
        )
        .expect("Failed to create worker2");

        // Start both workers
        worker1.start_async().await.expect("Failed to start worker1");
        worker2.start_async().await.expect("Failed to start worker2");

        let peer_id1 = worker1.get_peer_id().unwrap();
        let peer_id2 = worker2.get_peer_id().unwrap();

        println!("Worker1 peer ID: {}", peer_id1);
        println!("Worker2 peer ID: {}", peer_id2);

        // Worker1 sends message to Worker2
        let message = Message {
            data: b"Hello from Worker1!".to_vec(),
            peer_id: peer_id2.to_string(),
            multiaddrs: vec!["/ip4/127.0.0.1/tcp/8002".to_string()],
        };

        worker1
            .send_message(message)
            .await
            .expect("Failed to send message from worker1");

        // Give time for message delivery
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        // Worker2 should receive the message
        let received = worker2.get_next_message().await;
        assert!(received.is_some());

        let msg = received.unwrap();
        assert_eq!(msg.data, b"Hello from Worker1!");
        assert_eq!(msg.peer_id, peer_id1.to_string());

        // Clean shutdown
        worker1.stop_async().await.expect("Failed to stop worker1");
        worker2.stop_async().await.expect("Failed to stop worker2");
    }

    #[test(tokio::test)]
    async fn test_worker_without_blockchain() {
        // Test that worker fails gracefully when blockchain is not available
        let cancellation_token = CancellationToken::new();

        let mut worker = WorkerClientCore::new(
            0,
            "http://localhost:9999".to_string(), // Non-existent RPC
            Some("0x1234".to_string()),
            Some("0x5678".to_string()),
            Some(true),
            Some(1), // Low retry count for faster test
            cancellation_token,
            8003,
        )
        .expect("Failed to create worker");

        // Starting should fail due to blockchain connection issues
        let result = worker.start_async().await;
        assert!(result.is_err());
    }
} 