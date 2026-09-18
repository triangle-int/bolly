# Build the client and server from the same source as the release tag.
FROM node:22-bookworm AS client
WORKDIR /src/client
RUN corepack enable
COPY client/ ./
RUN pnpm install --frozen-lockfile && pnpm build

FROM rust:1.93.1-bookworm AS server
RUN apt-get update -qq && apt-get install -y --no-install-recommends \
    protobuf-compiler libssl-dev pkg-config clang cmake && \
    rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY server/ server/
# Cargo resolves all workspace members, even when building only the server.
COPY desktop/src-tauri/ desktop/src-tauri/
COPY --from=client /src/client/build/ client/build/
ARG GIT_HASH=dev
ENV GIT_HASH=${GIT_HASH}
RUN cargo build --locked --release -p server

# Chrome's Linux package limits this image to linux/amd64.
FROM ubuntu:24.04

# Install all runtime dependencies
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends \
      ca-certificates curl sudo procps git jq ffmpeg \
      python3 python3-pip python3-venv \
      fontconfig fonts-liberation fonts-dejavu-core && \
    # Node.js
    curl -fsSL https://deb.nodesource.com/setup_22.x | bash - > /dev/null 2>&1 && \
    apt-get install -y nodejs > /dev/null 2>&1 && \
    npm install -g pnpm > /dev/null 2>&1 && \
    # yt-dlp
    curl -fsSL https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp && chmod +x /usr/local/bin/yt-dlp && \
    # cloudflared
    DARCH=$(dpkg --print-architecture) && \
    curl -fsSL "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-${DARCH}.deb" -o /tmp/cf.deb && \
    dpkg -i /tmp/cf.deb && rm /tmp/cf.deb && \
    # gh CLI
    curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg 2>/dev/null && \
    echo "deb [arch=${DARCH} signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" > /etc/apt/sources.list.d/github-cli.list && \
    apt-get update -qq && apt-get install -y gh && \
    # Google Chrome (for chrome-devtools MCP server)
    curl -fsSL https://dl.google.com/linux/direct/google-chrome-stable_current_$(dpkg --print-architecture).deb -o /tmp/chrome.deb && \
    apt-get install -y /tmp/chrome.deb && rm /tmp/chrome.deb && \
    # Cleanup
    rm -rf /var/lib/apt/lists/* /tmp/*

# Keep executable code outside the persistent data volume.
COPY --from=server /src/target/release/server /usr/local/bin/bolly

ENV BOLLY_HOME=/data
ENV RUST_LOG=info,rig=warn
ENV BOLLY_CONTAINER=1
ENV PORT=26559

EXPOSE 26559
VOLUME /data

HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 \
    CMD curl --fail --silent "http://127.0.0.1:${PORT}/healthz" || exit 1

CMD ["/usr/local/bin/bolly"]
