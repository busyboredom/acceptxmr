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
  $ cd acceptxmr
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
    --network host \
    --mount type=bind,source=<database dir>,target=/AcceptXMR_DB \
    --mount type=bind,source=<TLS cert dir>,target=/cert \
    --mount type=bind,source=<config file path>,target=/acceptxmr.yaml \
    --env-file <env file path> \
    busyboredom/acceptxmr:latest
  ```
Note that the `acceptxmr.yaml` configuration file (described
[here](#Configuration)) applies directly to the bare `AcceptXMR-Server` service
running inside docker. The command in step (3) above will need to be adapted
appropriately if paths in `acceptxmr.yaml` are changed.

Click [here](../docker.sh) for an example command with paths filled out.

### Run with Docker Compose
1. Install Docker: https://docs.docker.com/get-docker/
2. Create a file called `docker-compose.yml` with the following contents,
   setting paths to whatever you desire:
  ```yaml
  name: acceptxmr
  services:
    server:
      image: busyboredom/acceptxmr:latest
      network_mode: "host"
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

Secrets should be configured via environment variable. Your private viewkey can
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

Interactive API documentation is available for each API at `<host>:<port>/
swagger-ui/` when running `AcceptXMR-Server`.

#### Internal API

The internal API serves endpoints which the end user should not have access to
(for example, creating and deletiing invoices).

**Create a new invoice: `POST /invoice`**

Example body:
```json
{
  "piconeros_due": 10000,
  "confirmations_required": 0,
  "expiration_in": 10,
  "order": "I am an example order",
  "callback": "https://example.com/payment",
}
```

Example response:
```json
{
  "invoice_id": "AAAAAAAAAEkAAAAAACXOWQ",
}
```

The `callback` field is optional, but if provided it will be called whenever
there is a change to the invoice's state (e.g. funds received, funds confirmed,
block height updated, etc.).

Example callback body:
```json
{
    "id": "_____wAAAAAAAAAAAAAAAA",
    "address": "84pKaXBd9biTwA7wihzUvrXN2YHoJBdFC4ZxEHQqaPuMFDa8Nyg1mywMXgzvjWBiTCfim7ZRfuJhvHavJrZ4Y7z3THW2Hmf",
    "uri": "monero:84pKaXBd9biTwA7wihzUvrXN2YHoJBdFC4ZxEHQqaPuMFDa8Nyg1mywMXgzvjWBiTCfim7ZRfuJhvHavJrZ4Y7z3THW2Hmf?tx_amount=0.000000001000",
    "amount_requested": 1000,
    "amount_paid": 0,
    "confirmations_required": 2,
    "confirmations": null,
    "expiration_in": 20,
    "current_height": 3130005,
    "order": "I am an example order",
    "callback": "https://example.com/payment"
}
```

**Delete an invoice: `DELETE /invoice?id=<invoice ID>`**

Stop tracking the specified invoice.

Response: `200`

**Get all invoice IDs: `GET /invoice/ids`**

Get all currently-tracked invoice IDs.

Example response:
```json
[
  "AAAAAAAAAAYAAAAAADCbqw",
  "AAAAAAAAALAAAAAAADCbqw"
]
```

#### External API

The external API serves endpoints which are safe to expose to the end user.
These endpoints do things like retrieve the status of an invoice or start a websocket connection.

**Get an invoice's status: `GET /invoice?id=<invoice id>`**

Example response:
```json
{
    "id": "_____wAAAAAAAAAAAAAAAA",
    "address": "84pKaXBd9biTwA7wihzUvrXN2YHoJBdFC4ZxEHQqaPuMFDa8Nyg1mywMXgzvjWBiTCfim7ZRfuJhvHavJrZ4Y7z3THW2Hmf",
    "uri": "monero:84pKaXBd9biTwA7wihzUvrXN2YHoJBdFC4ZxEHQqaPuMFDa8Nyg1mywMXgzvjWBiTCfim7ZRfuJhvHavJrZ4Y7z3THW2Hmf?tx_amount=0.000000000750",
    "amount_requested": 1000,
    "amount_paid": 250,
    "confirmations_required": 2,
    "confirmations": null,
    "expiration_in": 18,
    "current_height": 3130005,
    "order": "I am an example order",
    "callback": "https://example.com/payment"
}
```

**Subscribe to an invoice's updates via websocket: `GET /invoice/ws?id=<invoice ID>`**

Response: `101`

The updates received over websocket are identical the body returned by 
`GET /invoice?=<invoice ID>` described above.

**Go to payment UI: `GET /pay?id=<invoice ID>`**

Serves a minimal UI prompting the user for payment. The UI served uses [Tera]
(https://keats.github.io/tera/docs/#templates) templating and can be customized.

### Payment UI Templating

The available templates are `pay.html`, `missing-invoice.html`, and
`error.html`.

#### `pay.html`
  
`pay.html` is the template for prompting a user to pay a
currently-tracked invoice. Variables available to the template are:

| Variable | Type | Example |
| -------- | ---- | ------- |
| id | String | "_____wAAAAAAAAAAAAAAAA" |
| address | String | "84pKaXBd9biTwA7wihzUvrXN2YHoJBdFC4ZxEHQqaPuMFDa8Nyg1mywMXgzvjWBiTCfim7ZRfuJhvHavJrZ4Y7z3THW2Hmf" |
| uri | String | "monero:84pKaXBd9biTwA7wihzUvrXN2YHoJBdFC4ZxEHQqaPuMFDa8Nyg1mywMXgzvjWBiTCfim7ZRfuJhvHavJrZ4Y7z3THW2Hmf?tx_amount=0.000000000750" |
| amount_requested | u64 | 1000 |
| amount_paid | u64 | 250 |
| confirmations_required | u64 | 2 |
| confirmations | Option\<u64\> | null |
| expiration_in | u64 | 18 |
| current_height | u64 | 3130005 |
| order | String | "I am an example order" |
| callback | String | "https://example.com/payment" |

#### `missing-invoice.html`

`missing-invoice.html` is the template for informing a user that the requested
invoice is not available (likely because it expired). Variables available to the
template are:

| Variable | Type | Example |
| -------- | ---- | ------- |
| invoice_id | String | "_____wAAAAAAAAAAAAAAAA" |

#### `error.html`

`error.html` is the template displayed when an internal error occurs. Variables
available to the template are:

| Variable | Type | Example |
| -------- | ---- | ------- |
| error | String | "Failed to start the flux capacitor" |
