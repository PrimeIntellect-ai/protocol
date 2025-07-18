FROM ubuntu:22.04

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY release-artifacts/discovery-linux-x86_64 /usr/local/bin/discovery
RUN chmod +x /usr/local/bin/discovery

ENV RPC_URL="http://localhost:8545"
ENV PLATFORM_API_KEY="prime"
ENV REDIS_URL="redis://localhost:6380"
ENV PORT="8089"
ENV MAX_NODES_PER_IP="3"
ENV MODE="full"
ENV LOCATION_SERVICE_URL=""
ENV LOCATION_SERVICE_API_KEY=""

RUN echo '#!/bin/sh\n\
exec /usr/local/bin/discovery \
--rpc-url "$RPC_URL" \
--platform-api-key "$PLATFORM_API_KEY" \
--redis-url "$REDIS_URL" \
--port "$PORT" \
--max-nodes-per-ip "$MAX_NODES_PER_IP" \
--mode "$MODE" \
$([ ! -z "$LOCATION_SERVICE_URL" ] && echo "--location-service-url $LOCATION_SERVICE_URL") \
$([ ! -z "$LOCATION_SERVICE_API_KEY" ] && echo "--location-service-api-key $LOCATION_SERVICE_API_KEY") \
"$@"' > /entrypoint.sh && \
chmod +x /entrypoint.sh

EXPOSE 8089
ENTRYPOINT ["/entrypoint.sh"]