FROM rust:1.82-alpine as backend-builder
WORKDIR /usr/src/NATS-WebUI
COPY . .
RUN apk update && apk upgrade 
RUN apk add build-base openssl-dev sqlite-dev git
RUN CARGO_NET_GIT_FETCH_WITH_CLI=true \
    RUSTFLAGS="-C target-feature=-crt-static" \
    cargo build --release && \
    strip target/release/nats-webui 

FROM node:lts as frontend-builder
WORKDIR /usr/src/NATS-WebUI
COPY . .
WORKDIR /usr/src/NATS-WebUI/web
RUN npm i
RUN npm update
RUN npm run build --release

FROM alpine:3.18
MAINTAINER Jens Thiel <thielj@gmail.com>
WORKDIR /opt/nats
VOLUME /data
RUN apk update && apk upgrade && \
    apk add --no-cache libgcc libssl1.1 libcrypto1.1 ca-certificates sqlite-libs && \
    update-ca-certificates && \
    mkdir -p /opt/nats/web/dist && \
    mkdir /data && chown 1000:1000 /data 
COPY --from=backend-builder  /usr/src/NATS-WebUI/target/release/nats-webui /opt/nats/nats-webui
COPY --from=frontend-builder /usr/src/NATS-WebUI/web/dist/                 /opt/nats/web/dist
USER 1000:1000
EXPOSE 80
ENTRYPOINT ["/opt/nats/nats-webui"]
