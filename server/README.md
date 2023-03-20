[![BuildStatus](https://github.com/busyboredom/acceptxmr/workflows/CI/badge.svg)](https://img.shields.io/github/actions/workflow/status/busyboredom/acceptxmr/ci.yml?branch=main)
[![Docker Image Size](https://badgen.net/docker/size/busyboredom/acceptxmr/latest/amd64?icon=docker&label=Size)](https://hub.docker.com/r/busyboredom/acceptxmr/)

# `AcceptXMR-Server`: A monero payment gateway.
`AcceptXMR-Server` is a batteries-included monero payment gateway built around
the AcceptXMR library.

If your application requires more flexibility than `AcceptXMR-Server` offers,
please see the [`AcceptXMR`](../library/) library instead.

## Getting Started
### Build and Run from Source
1. Install rust: https://www.rust-lang.org/tools/install
2. Clone this repository:
  ```
  git clone https://github.com/busyboredom/acceptxmr.git
  ```
3. Run it:
  ```
  cargo run --release
  ```

### Run with Docker
1. Install Docker: https://docs.docker.com/get-docker/
2. Pull the latest AcceptXMR image:
  ```
  docker pull busyboredom/acceptxmr:latest
  ```
3. Run it (setting port and database directory to whatever you desire): 
  ```
  docker run -d \
    --name acceptxmr \
    --restart=always \
    -p <port>:8080 \
    --mount type=bind,source=<database dir>,target=/AcceptXMR_DB \
    busyboredom/acceptxmr:latest
  ```
4. That's it, you are now serving a payment gateway at `localhost:<port>`.

### Run with Docker Compose
1. Install Docker: https://docs.docker.com/get-docker/
2. Create a file called `docker-compose.yml` with the following contents,
   setting port to whatever you desire:
  ```
  name: acceptxmr
  services:
    server:
      image: busyboredom/acceptxmr:latest
      ports:
        - "<port>:8080"
      volumes:
        - db:/AcceptXMR_DB
      restart: always
  volumes:
    db:
  ```
3. Run it:
  ```
  docker compose up -d
  ```
4. That's it, you are now serving a payment gateway at `localhost:<port>`.

### Configuration
TODO

### API
TODO
