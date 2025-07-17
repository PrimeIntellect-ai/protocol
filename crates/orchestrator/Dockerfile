FROM ubuntu:22.04

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY release-artifacts/orchestrator-linux-x86_64 /usr/local/bin/orchestrator
RUN chmod +x /usr/local/bin/orchestrator

# Default environment variables based on Args struct
ENV RPC_URL="http://localhost:8545"
ENV COORDINATOR_KEY=""
ENV COMPUTE_POOL_ID="0"
ENV DOMAIN_ID="0"
ENV PORT="8090"
ENV URL=""
ENV HOST=""
ENV DISCOVERY_REFRESH_INTERVAL="10"
ENV REDIS_STORE_URL="redis://localhost:6380"
ENV BOOTNODES="/ip4/127.0.0.1/tcp/4005/p2p/12D3KooWJj3haDEzxGSbGSAvXCiE9pDYC9xHDdtQe8B2donhfwXL"
ENV ADMIN_API_KEY="admin"
ENV DISABLE_EJECTION="false"
ENV S3_CREDENTIALS=""
ENV BUCKET_NAME=""
ENV LOG_LEVEL=""
ENV HOURLY_S3_UPLOAD_LIMIT="2"
ENV MODE="full"
ENV NODE_GROUP_CONFIGS=""
ENV NODE_GROUP_MANAGEMENT_INTERVAL="10"
ENV WEBHOOK_CONFIGS=""
ENV MAX_HEALTHY_NODES_WITH_SAME_ENDPOINT="1"

RUN echo '#!/bin/sh\n\
exec /usr/local/bin/orchestrator \
--rpc-url "$RPC_URL" \
--coordinator-key "$COORDINATOR_KEY" \
--compute-pool-id "$COMPUTE_POOL_ID" \
--domain-id "$DOMAIN_ID" \
--port "$PORT" \
$([ ! -z "$URL" ] && echo "--url $URL") \
$([ ! -z "$HOST" ] && echo "--host $HOST") \
--discovery-refresh-interval "$DISCOVERY_REFRESH_INTERVAL" \
--redis-store-url "$REDIS_STORE_URL" \
--bootnodes "$BOOTNODES" \
--admin-api-key "$ADMIN_API_KEY" \
--mode "$MODE" \
$([ "$DISABLE_EJECTION" = "true" ] && echo "--disable-ejection") \
$([ ! -z "$BUCKET_NAME" ] && echo "--bucket-name $BUCKET_NAME") \
$([ ! -z "$LOG_LEVEL" ] && echo "--log-level $LOG_LEVEL") \
$([ ! -z "$HOURLY_S3_UPLOAD_LIMIT" ] && echo "--hourly-s3-upload-limit $HOURLY_S3_UPLOAD_LIMIT") \
$([ ! -z "$NODE_GROUP_MANAGEMENT_INTERVAL" ] && echo "--node-group-management-interval $NODE_GROUP_MANAGEMENT_INTERVAL") \
$([ ! -z "$MAX_HEALTHY_NODES_WITH_SAME_ENDPOINT" ] && echo "--max-healthy-nodes-with-same-endpoint $MAX_HEALTHY_NODES_WITH_SAME_ENDPOINT") \
"$@"' > /entrypoint.sh && \
chmod +x /entrypoint.sh

EXPOSE 8090
ENTRYPOINT ["/entrypoint.sh"]
