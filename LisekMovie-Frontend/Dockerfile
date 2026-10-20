# LisekMovie web app: SvelteKit (adapter-node) served by server.js, which
# also proxies /api (HTTP + WebSocket) to lisek-server.
#
#   docker build -t lisekmovie/web LisekMovie-Frontend
#   docker run -p 3000:3000 -e LISEK_API_URL=http://lisek-server:8080 lisekmovie/web

ARG NODE_VERSION=24

FROM node:${NODE_VERSION}-trixie-slim AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund
COPY . .
RUN npm run build \
    && npm prune --omit=dev --no-audit --no-fund

FROM node:${NODE_VERSION}-trixie-slim AS runtime
ENV NODE_ENV=production \
    PORT=3000 \
    HOST=0.0.0.0 \
    LISEK_API_URL=http://lisek-server:8080
WORKDIR /app
COPY --from=build --chown=node:node /app/package.json ./
COPY --from=build --chown=node:node /app/node_modules ./node_modules
COPY --from=build --chown=node:node /app/build ./build
COPY --chown=node:node server.js ./
USER node
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD node -e "fetch('http://127.0.0.1:3000/healthz').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"
CMD ["node", "server.js"]
