FROM rust:1.71-slim-bookworm as build

# Create a new empty shell project.
RUN USER=root cargo new --bin acceptxmr-server
WORKDIR /acceptxmr-server

# Copy over the manifests
COPY ./.cargo ./.cargo
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
COPY ./library/Cargo.toml ./library/Cargo.toml
# Create main.rs so build succeeds.
RUN cargo init server
RUN rm ./server/Cargo.toml
COPY ./server/Cargo.toml ./server/Cargo.toml

# Copy over the AcceptXMR lib.
COPY ./library ./library

# This build step will cache the dependencies (including the AcceptXMR lib).
RUN cargo build --release
RUN rm ./server/src/*.rs

# Copy the source tree.
COPY ./server/src ./server/src

# Build for release.
RUN rm ./target/release/deps/acceptxmr_server*
RUN cargo build --release

# Final base.
FROM frolvlad/alpine-glibc:alpine-3.17

# Copy the binary from the build stage.
COPY --from=build /acceptxmr-server/target/release/acceptxmr-server .

# Copy the static files.
COPY ./server/static ./server/static

# Add metadata that the container will listen to port 8080.
EXPOSE 8080

# Set an environment variable so the AcceptXMR knows it's in a docker container.
ENV DOCKER=true

# Set the startup command to run the binary.
CMD ["./acceptxmr-server"]
