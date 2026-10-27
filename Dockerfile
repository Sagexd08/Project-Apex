# Atlas MVP image: static musl binary on an empty base. No shell, no package manager, no root.
#   docker build -t atlas-mvp .
#   docker run --rm -p 8080:8080 -e ATLAS_API_KEYS=demo=change-me-please -v atlas-data:/data atlas-mvp
FROM rust:1.95-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# Size flags via env so the workspace manifest (and its release profile) stays untouched.
ENV CARGO_PROFILE_RELEASE_STRIP=true \
    CARGO_PROFILE_RELEASE_LTO=true \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
RUN cargo build --release --locked -p atlas-cli \
 && mkdir /data && chown 10001:10001 /data

FROM scratch
COPY --from=build /src/target/release/atlas /atlas
COPY --from=build --chown=10001:10001 /data /data
USER 10001:10001
EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["/atlas"]
# --dim must match the embedding size you store; 384 is only a default. ATLAS_API_KEYS is required.
CMD ["serve", "--dir", "/data", "--dim", "384", "--addr", "0.0.0.0:8080"]
