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
  ```bash
  $ git clone https://github.com/busyboredom/acceptxmr.git 
  $ cd acceptxmr && 
  ```
3. Run it:
  ```bash
  $ cargo run --release
  ```

### Run with Docker
1. Install Docker: https://docs.docker.com/get-docker/
2. Pull the latest AcceptXMR image:
  ```bash
  $ docker pull busyboredom/acceptxmr:latest
  ```
3. Run it (setting ports and paths to whatever you desire): 
  ```bash
  $ docker run -d \
    --name acceptxmr \
    --restart=always \
    -p <external API port>:8080 \
    -p <internal API port>:8081 \
    --mount type=bind,source=<database dir>,target=/AcceptXMR_DB \
    --mount type=bind,source=<TLS cert dir>,target=/cert \
    --mount type=bind,source=<config file path>,target=/acceptxmr.yaml \
    --env-file <env file path> \
    busyboredom/acceptxmr:latest
  ```
Note that the `acceptxmr.yaml` configuration file (described
[here](#Configuration)) applies directly to the bare `AcceptXMR-Server` service
running inside docker. The command in step (3) above will need to be adapted
appropriately if ports or paths in `acceptxmr.yaml` are changed.

Click [here](../docker.sh) for an example command with paths and ports filled
out.

### Run with Docker Compose
1. Install Docker: https://docs.docker.com/get-docker/
2. Create a file called `docker-compose.yml` with the following contents,
   setting ports and paths to whatever you desire:
  ```yaml
  name: acceptxmr
  services:
    server:
      image: busyboredom/acceptxmr:latest
      ports:
        - "<external API port>:8080"
        - "<internal API port>:8081"
      volumes:
        - db:/AcceptXMR_DB
        - <path to config file>:/acceptxmr.yaml
        - <path to certificate dir>:/cert
      env_file: <path to env file>
      restart: always
  volumes:
    db:
  ```
3. Run it:
  ```bash
  $ docker compose up -d
  ```

Note that the `acceptxmr.yaml` configuration file (described
[here](#Configuration)) applies directly to the bare `AcceptXMR-Server` service
running inside docker. The file in step (2) above will need to be adapted
appropriately if ports or paths in `acceptxmr.yaml` are changed.

Click [here](../docker-compose.yml) for an example `docker-compose.yml` with
paths and ports filled out. 

### Configuration
`AcceptXMR-Server` uses a configuration file named `acceptxmr.yaml` for most
configuration, and uses environment variables for secrets. 

The location of `acceptxmr.yaml` is expected to be the current directory by
default. An alternative location can be specified by passing the `--config-file
<path/to/file.yaml>` command line argument, or by setting the `CONFIG_FILE`
environment variable.

Please click [here](../acceptxmr.yaml) for an example of what can be configured
in `acceptxmr.yaml`.

Secrets should be configured via environment variable. Your priviate viewkey can
be set using the `PRIVATE_VIEWKEY` environment variable, and bearer
authentication tokens can be set using the `INTERNAL_API_TOKEN` and
`EXTERNAL_API_TOKEN` variables if desired. 

Please click [here](../.env) for an example of how to configure secrets in a
`.env` file.

### API

`AcceptXMR-Server` serves two APIs. The first is an "internal" API meant to be
used server-side (i.e. not exposed to the internet). The second API is an
"external" API, which is safe to expose to the end-user (i.e. it may be exposed
to the internet).

#### Internal API

The internal API serves endpoints which the end user should not have access to
(for example, creating new invoices).

**Create a new invoice: `POST /invoice`**

Example body:
```json
{
  piconeros_due: 10000,
  confirmations_required: 0,
  expiration_in: 10,
  description: "I am an example description",
}
```

Example response:
```json
{
  invoice_id: "748934525",
}
```

#### External API

The external API serves endpoints which are safe to expose to the end user.
These endpoints do things like retreive the status of an invoice, cancel an
invoice, or start a websocket connection.

**Get an invoice's status: `GET /invoice?id=<invoice id>`**

Example response:
```json
{
  address: "84pKaXBd9biTwA7wihzUvrXN2YHoJBdFC4ZxEHQqaPuMFDa8Nyg1mywMXgzvjWBiTCfim7ZRfuJhvHavJrZ4Y7z3THW2Hmf",
  amount_paid: 4000,
  amount_requested: 5000,
  uri: "monero:84pKaXBd9biTwA7wihzUvrXN2YHoJBdFC4ZxEHQqaPuMFDa8Nyg1mywMXgzvjWBiTCfim7ZRfuJhvHavJrZ4Y7z3THW2Hmf?tx_amount=0.000000001000"
  confirmations: 1,
  confirmations_required: 2,
  expiration_in: 4,
  description: "I am an example description",
}
```

**Subscribe to an invoice's updates via websocket: `GET /invoice/ws?id=<invoice ID>`**

Response: *Whatever the upgrade status code is.*

**Cancel an invoice: `DELETE /invoice?id=<invoice ID>`**

Response: `200`

**Go to payment UI: `GET /pay?id=<invoice ID>`**

Serves a minimal UI prompting the user for payment.
