FROM ubuntu:22.04

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY release-artifacts/validator-linux-x86_64 /usr/local/bin/validator
RUN chmod +x /usr/local/bin/validator

ENV RPC_URL="http://localhost:8545"
ENV VALIDATOR_KEY=""
ENV DISCOVERY_URL="http://localhost:8089"

RUN echo '#!/bin/sh\n\
exec /usr/local/bin/validator \
--rpc-url "$RPC_URL" \
--validator-key "$VALIDATOR_KEY" \
--discovery-url "$DISCOVERY_URL" \
"$@"' > /entrypoint.sh && \
chmod +x /entrypoint.sh

EXPOSE 8080
ENTRYPOINT ["/entrypoint.sh"]