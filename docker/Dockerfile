# Base FIPA node image — the node engine (mesh-node). The *agent* is added at
# run time via FIPA_AGENT (df|ams|pa|bs|ba); identity persists in the /data volume.
#
#   docker build -t fipa-node:base -f docker/Dockerfile .

FROM rust:1-bookworm AS build
# protoc: fipa-wasm-agents' build.rs runs tonic/prost protobuf codegen.
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --bin mesh-node

FROM debian:bookworm-slim
RUN useradd -m fipa && mkdir -p /data && chown fipa /data
COPY --from=build /src/target/release/mesh-node /usr/local/bin/mesh-node
USER fipa
ENV FIPA_BIND=0.0.0.0:9000 FIPA_DATA=/data
EXPOSE 9000
VOLUME ["/data"]
ENTRYPOINT ["mesh-node"]
