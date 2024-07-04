FROM --platform=$BUILDPLATFORM rust:1.76-slim-bookworm as build

# Create a new empty shell project.
RUN USER=root cargo new --bin acceptxmr-server
WORKDIR /acceptxmr-server

# Prepare for cross compilation.
ARG TARGETARCH
RUN apt update && apt upgrade -y 

RUN if [ "$TARGETARCH" = "arm64" ]; then \
        apt install -y g++-aarch64-linux-gnu libc6-dev-arm64-cross; \
        rustup toolchain install stable-aarch64-unknown-linux-gnu; \
        rustup target add aarch64-unknown-linux-gnu; \
    elif [ "$TARGETARCH" = "amd64" ]; then \
        apt install -y g++-x86_64-linux-gnu libc6-dev-amd64-cross; \
        rustup toolchain install stable-x86_64-unknown-linux-gnu; \
        rustup target add x86_64-unknown-linux-gnu; \
    else \
        echo "Unsupported target arch $TARGETARCH"; \
        exit 1; \
    fi

# Copy over the manifests
COPY ./.cargo ./.cargo
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
COPY ./library/Cargo.toml ./library/Cargo.toml
COPY ./testing-utils/Cargo.toml ./testing-utils/Cargo.toml
# Create main.rs so build succeeds.
RUN cargo init server
RUN touch server/src/lib.rs
RUN rm ./server/Cargo.toml
COPY ./server/Cargo.toml ./server/Cargo.toml

# Copy over the AcceptXMR lib.
COPY ./library ./library

# Copy over the testing-utils lib.
COPY ./testing-utils ./testing-utils

# This build step will cache the dependencies (including the AcceptXMR lib and testing-utils).
RUN if [ "$TARGETARCH" = "arm64" ]; then \
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=/usr/bin/aarch64-linux-gnu-gcc cargo build --target=aarch64-unknown-linux-gnu --release; \
    elif [ "$TARGETARCH" = "amd64" ]; then \
        CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=/usr/bin/x86_64-linux-gnu-gcc cargo build --target=x86_64-unknown-linux-gnu --release; \
    else \
        echo "Unsupported target arch $TARGETARCH"; \
        exit 1; \
    fi

# Copy the source tree.
RUN rm ./server/src/*.rs
COPY ./server/src ./server/src

# Build for release.
RUN if [ "$TARGETARCH" = "arm64" ]; then \
        rm ./target/aarch64-unknown-linux-gnu/release/deps/acceptxmr_server*; \
        rm ./target/aarch64-unknown-linux-gnu/release/deps/libacceptxmr_server*; \
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=/usr/bin/aarch64-linux-gnu-gcc cargo build --target=aarch64-unknown-linux-gnu --release; \
        cp /acceptxmr-server/target/aarch64-unknown-linux-gnu/release/acceptxmr-server .; \
    elif [ "$TARGETARCH" = "amd64" ]; then \
        rm ./target/x86_64-unknown-linux-gnu/release/deps/acceptxmr_server*; \
        rm ./target/x86_64-unknown-linux-gnu/release/deps/libacceptxmr_server*; \
        CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=/usr/bin/x86_64-linux-gnu-gcc cargo build --target=x86_64-unknown-linux-gnu --release; \
        cp /acceptxmr-server/target/x86_64-unknown-linux-gnu/release/acceptxmr-server .; \
    else \
        echo "Unsupported target arch $TARGETARCH"; \
        exit 1; \
    fi

# Final base.
FROM frolvlad/alpine-glibc:alpine-3.17

# Copy the binary from the build stage.
COPY --from=build /acceptxmr-server/acceptxmr-server .

# Copy the static files.
COPY ./server/static ./server/static

# Add metadata that the container will listen to port 8080 and 8081.
EXPOSE 8080
EXPOSE 8081

# Set the startup command to run the binary.
CMD ["./acceptxmr-server"]
