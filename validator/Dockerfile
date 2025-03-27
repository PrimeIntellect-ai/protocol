FROM ubuntu:22.04

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY release-artifacts/validator-linux-x86_64 /usr/local/bin/validator
RUN chmod +x /usr/local/bin/validator

ENV RPC_URL="http://localhost:8545"
ENV VALIDATOR_KEY=""
ENV DISCOVERY_URL="http://localhost:8089"
ENV POOL_ID=""
ENV WORK_VALIDATION_INTERVAL="30"
ENV LEVITICUS_URL=""
ENV LEVITICUS_TOKEN=""
ENV S3_CREDENTIALS=""
ENV BUCKET_NAME=""
ENV LOG_LEVEL=""


RUN echo '#!/bin/sh\n\
exec /usr/local/bin/validator \
--rpc-url "$RPC_URL" \
--validator-key "$VALIDATOR_KEY" \
--discovery-url "$DISCOVERY_URL" \
--pool-id "$POOL_ID" \
--work-validation-interval "$WORK_VALIDATION_INTERVAL" \
--leviticus-url "$LEVITICUS_URL" \
$([ ! -z "$LEVITICUS_TOKEN" ] && echo "--leviticus-token $LEVITICUS_TOKEN") \
$([ ! -z "$S3_CREDENTIALS" ] && echo "--s3-credentials $S3_CREDENTIALS") \
$([ ! -z "$BUCKET_NAME" ] && echo "--bucket-name $BUCKET_NAME") \
$([ ! -z "$LOG_LEVEL" ] && echo "--log-level $LOG_LEVEL") \
"$@"' > /entrypoint.sh && \
chmod +x /entrypoint.sh

EXPOSE 8080
ENTRYPOINT ["/entrypoint.sh"]