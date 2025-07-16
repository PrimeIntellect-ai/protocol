FROM ubuntu:22.04

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY release-artifacts/bootnode-linux-x86_64 /usr/local/bin/bootnode
RUN chmod +x /usr/local/bin/bootnode

ENV LIBP2P_PRIVATE_KEY=""
ENV LIBP2P_PORT="4005"
ENV LOG_LEVEL="info"

RUN echo '#!/bin/sh\n\
exec /usr/local/bin/bootnode \
--libp2p-private-key "$LIBP2P_PRIVATE_KEY" \
--libp2p-port "$LIBP2P_PORT" \
--log-level "$LOG_LEVEL" \
"$@"' > /entrypoint.sh && \
chmod +x /entrypoint.sh

EXPOSE 4005
ENTRYPOINT ["/entrypoint.sh"]