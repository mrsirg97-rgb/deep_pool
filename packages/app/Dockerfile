# syntax=docker/dockerfile:1.6
#
# Build context: repository root (pnpm workspace + all packages).
# Build command:  docker compose --profile full build app
#
# Three stages:
#   1. deps    — installs the full pnpm workspace (cached when lockfile unchanged)
#   2. builder — runs `next build` against the standalone target
#   3. runtime — minimal node:alpine image with the standalone bundle only
#
# Next.js standalone output bundles only the deps actually imported, so the
# runtime image ships ~150MB instead of ~800MB+ for a full node_modules copy.

# --------------------------------------------------------------------
# Stage 1: deps — install the pnpm workspace
# --------------------------------------------------------------------
FROM node:20-alpine AS deps

# pnpm via corepack — the version is pinned by the lockfile's metadata
# so there's no need to install a specific version explicitly.
RUN corepack enable

WORKDIR /workspace

# Bring in just the files needed to resolve + install dependencies.
# Copying packages/*/package.json (without source) means the install
# layer is cached as long as no package.json or lockfile changes.
COPY pnpm-workspace.yaml pnpm-lock.yaml package.json* ./
COPY packages/app/package.json packages/app/
COPY packages/sdk/package.json packages/sdk/

RUN --mount=type=cache,target=/root/.local/share/pnpm/store,sharing=locked \
    pnpm install --frozen-lockfile

# --------------------------------------------------------------------
# Stage 2: builder — build the UI with standalone output
# --------------------------------------------------------------------
FROM node:20-alpine AS builder

RUN corepack enable
WORKDIR /workspace

# Reuse the resolved node_modules from deps; copy source on top.
# The app imports `deeppoolsdk` from npm (not the workspace `packages/sdk/`),
# so we don't need the workspace SDK's node_modules in the builder. The root
# node_modules already holds the published `deeppoolsdk` package via
# pnpm install above, and app's node_modules has its direct deps.
COPY --from=deps /workspace/node_modules ./node_modules
COPY --from=deps /workspace/packages/app/node_modules ./packages/app/node_modules
COPY pnpm-workspace.yaml pnpm-lock.yaml package.json* ./
COPY packages/app/ packages/app/

# NEXT_PUBLIC_* values are inlined into the client bundle at build time.
# Defaults match the loopback-only compose setup; override at build time
# (e.g. for Vercel) by passing --build-arg or overriding in compose.
ARG NEXT_PUBLIC_RPC_URL=https://api.devnet.solana.com
ARG NEXT_PUBLIC_INDEXER_URL=http://localhost:8080
ARG NEXT_PUBLIC_PROGRAM_ID=4GFr4EdVn9TGArzSaCraaGXNDj8sgf4nNRXH3LkrC8Q9
ENV NEXT_PUBLIC_RPC_URL=${NEXT_PUBLIC_RPC_URL} \
    NEXT_PUBLIC_INDEXER_URL=${NEXT_PUBLIC_INDEXER_URL} \
    NEXT_PUBLIC_PROGRAM_ID=${NEXT_PUBLIC_PROGRAM_ID} \
    NEXT_TELEMETRY_DISABLED=1

RUN pnpm --filter deep-pool-app build

# --------------------------------------------------------------------
# Stage 3: runtime — minimal image with the standalone bundle
# --------------------------------------------------------------------
FROM node:20-alpine AS runtime

WORKDIR /app

# Run as a non-root user. node:alpine ships a `node` user (uid 1000).
ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    PORT=3000 \
    HOSTNAME=0.0.0.0

# Standalone output places a self-contained server.js at the root of the
# package's standalone dir, with .next/static and public served beside it.
COPY --from=builder --chown=node:node \
    /workspace/packages/app/.next/standalone ./
COPY --from=builder --chown=node:node \
    /workspace/packages/app/.next/static ./packages/app/.next/static

USER node
EXPOSE 3000

# server.js is emitted by Next.js standalone at the package root.
CMD ["node", "packages/app/server.js"]
