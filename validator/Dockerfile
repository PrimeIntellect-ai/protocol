FROM ubuntu:22.04

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY release-artifacts/validator-linux-x86_64 /usr/local/bin/validator
RUN chmod +x /usr/local/bin/validator

ENV RPC_URL="http://localhost:8545"
ENV VALIDATOR_KEY=""
ENV DISCOVERY_URL="http://localhost:8089"
ENV POOL_ID=""
ENV WORK_VALIDATION_INTERVAL="30"
ENV TOPLOC_SERVER_URL=""
ENV TOPLOC_AUTH_TOKEN=""
ENV S3_CREDENTIALS=""
ENV BUCKET_NAME=""
ENV LOG_LEVEL=""
ENV REDIS_URL="redis://localhost:6380"
ENV TOPLOC_GRACE_INTERVAL="15"
ENV TOPLOC_WORK_VALIDATION_INTERVAL="15"

RUN echo '#!/bin/sh\n\
exec /usr/local/bin/validator \
--rpc-url "$RPC_URL" \
--validator-key "$VALIDATOR_KEY" \
--discovery-url "$DISCOVERY_URL" \
--pool-id "$POOL_ID" \
--work-validation-interval "$WORK_VALIDATION_INTERVAL" \
--toploc-server-url "$TOPLOC_SERVER_URL" \
$([ ! -z "$TOPLOC_AUTH_TOKEN" ] && echo "--toploc-auth-token $TOPLOC_AUTH_TOKEN") \
$([ ! -z "$S3_CREDENTIALS" ] && echo "--s3-credentials $S3_CREDENTIALS") \
$([ ! -z "$BUCKET_NAME" ] && echo "--bucket-name $BUCKET_NAME") \
$([ ! -z "$LOG_LEVEL" ] && echo "--log-level $LOG_LEVEL") \
$([ ! -z "$REDIS_URL" ] && echo "--redis-url $REDIS_URL") \
$([ ! -z "$TOPLOC_GRACE_INTERVAL" ] && echo "--toploc-grace-interval $TOPLOC_GRACE_INTERVAL") \
$([ ! -z "$TOPLOC_WORK_VALIDATION_INTERVAL" ] && echo "--toploc-work-validation-interval $TOPLOC_WORK_VALIDATION_INTERVAL") \
"$@"' > /entrypoint.sh && \
chmod +x /entrypoint.sh

EXPOSE 8080
ENTRYPOINT ["/entrypoint.sh"]