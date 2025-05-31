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
ENV DISCOVERY_URL="http://localhost:8089"
ENV ADMIN_API_KEY="admin"
ENV DISABLE_EJECTION="false"
ENV S3_CREDENTIALS=""
ENV BUCKET_NAME=""
ENV LOG_LEVEL=""
ENV HOURLY_S3_UPLOAD_LIMIT="2"
ENV WEBHOOK_URLS=""
ENV MODE="full"
ENV NODE_GROUP_CONFIGS=""
ENV NODE_GROUP_MANAGEMENT_INTERVAL="10"

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
--discovery-url "$DISCOVERY_URL" \
--admin-api-key "$ADMIN_API_KEY" \
--mode "$MODE" \
$([ "$DISABLE_EJECTION" = "true" ] && echo "--disable-ejection") \
$([ ! -z "$BUCKET_NAME" ] && echo "--bucket-name $BUCKET_NAME") \
$([ ! -z "$LOG_LEVEL" ] && echo "--log-level $LOG_LEVEL") \
$([ ! -z "$HOURLY_S3_UPLOAD_LIMIT" ] && echo "--hourly-s3-upload-limit $HOURLY_S3_UPLOAD_LIMIT") \
$([ ! -z "$WEBHOOK_URLS" ] && echo "--webhook-urls $WEBHOOK_URLS") \
$([ ! -z "$NODE_GROUP_MANAGEMENT_INTERVAL" ] && echo "--node-group-management-interval $NODE_GROUP_MANAGEMENT_INTERVAL") \
"$@"' > /entrypoint.sh && \
chmod +x /entrypoint.sh

EXPOSE 8090
ENTRYPOINT ["/entrypoint.sh"]