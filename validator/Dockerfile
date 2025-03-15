FROM ubuntu:22.04

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY release-artifacts/validator-linux-x86_64 /usr/local/bin/validator
RUN chmod +x /usr/local/bin/validator

ENV RPC_URL="http://localhost:8545"
ENV VALIDATOR_KEY=""
ENV DISCOVERY_URL="http://localhost:8089"
ENV WORK_VALIDATION_CONTRACT=""
ENV POOL_ID=""
ENV WORK_VALIDATION_INTERVAL="30"
ENV LEVITICUS_URL=""

RUN echo '#!/bin/sh\n\
exec /usr/local/bin/validator \
--rpc-url "$RPC_URL" \
--validator-key "$VALIDATOR_KEY" \
--discovery-url "$DISCOVERY_URL" \
--work-validation-contract "$WORK_VALIDATION_CONTRACT" \
--pool-id "$POOL_ID" \
--work-validation-interval "$WORK_VALIDATION_INTERVAL" \
--leviticus-url "$LEVITICUS_URL" \
"$@"' > /entrypoint.sh && \
chmod +x /entrypoint.sh

EXPOSE 8080
ENTRYPOINT ["/entrypoint.sh"]