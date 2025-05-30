FROM ubuntu:22.04

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY release-artifacts/validator-linux-x86_64 /usr/local/bin/validator
RUN chmod +x /usr/local/bin/validator

ENV RPC_URL="http://localhost:8545"
ENV VALIDATOR_KEY=""
ENV DISCOVERY_URL="http://localhost:8089"
ENV POOL_ID=""
ENV S3_CREDENTIALS=""
ENV BUCKET_NAME=""
ENV LOG_LEVEL=""
ENV REDIS_URL="redis://localhost:6380"
ENV TOPLOC_GRACE_INTERVAL="15"
ENV TOPLOC_WORK_VALIDATION_INTERVAL="15"
ENV TOPLOC_WORK_VALIDATION_UNKNOWN_STATUS_EXPIRY_SECONDS="120"
ENV TOPLOC_CONFIGS=""
ENV DISABLE_HARDWARE_VALIDATION="false"
ENV DISABLE_TOPLOC_INVALIDATION="false"
ENV VALIDATOR_PENALTY="1"
ENV USE_GROUPING="false"

RUN echo '#!/bin/sh\n\
exec /usr/local/bin/validator \
--rpc-url "$RPC_URL" \
--validator-key "$VALIDATOR_KEY" \
--discovery-url "$DISCOVERY_URL" \
$([ ! -z "$POOL_ID" ] && echo "--pool-id $POOL_ID") \
$([ ! -z "$BUCKET_NAME" ] && echo "--bucket-name $BUCKET_NAME") \
$([ ! -z "$LOG_LEVEL" ] && echo "--log-level $LOG_LEVEL") \
$([ ! -z "$REDIS_URL" ] && echo "--redis-url $REDIS_URL") \
$([ ! -z "$TOPLOC_GRACE_INTERVAL" ] && echo "--toploc-grace-interval $TOPLOC_GRACE_INTERVAL") \
$([ ! -z "$TOPLOC_WORK_VALIDATION_INTERVAL" ] && echo "--toploc-work-validation-interval $TOPLOC_WORK_VALIDATION_INTERVAL") \
$([ ! -z "$TOPLOC_WORK_VALIDATION_UNKNOWN_STATUS_EXPIRY_SECONDS" ] && echo "--toploc-work-validation-unknown-status-expiry-seconds $TOPLOC_WORK_VALIDATION_UNKNOWN_STATUS_EXPIRY_SECONDS") \
$([ ! -z "$USE_GROUPING" ] && echo "--use-grouping") \
$([ ! -z "$DISABLE_TOPLOC_INVALIDATION" ] && echo "--disable-toploc-invalidation") \
$([ ! -z "$VALIDATOR_PENALTY" ] && echo "--validator-penalty $VALIDATOR_PENALTY") \
$([ "$DISABLE_HARDWARE_VALIDATION" = "true" ] && echo "--disable-hardware-validation") \
"$@"' > /entrypoint.sh && \
chmod +x /entrypoint.sh

EXPOSE 9879
ENTRYPOINT ["/entrypoint.sh"]