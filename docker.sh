#! /bin/bash

# To use this command:
#   1. Install Docker: https://docs.docker.com/get-docker/
#   2. Clone this repository:
#       $ git clone https://github.com/busyboredom/acceptxmr.git && cd acceptxmr
#   3. Build the image:
#       $ docker build . -t acceptxmr
#   4. Run it:
#       $ sh docker.sh
#
# This command builds AcceptXMR-Server locally instead of pulling it from docker
# hub.

docker run \
    --name acceptxmr \
    -p 8080:8080 \
    -p 8081:8081 \
    --mount type=bind,source=${PWD}/AcceptXMR_DB,target=/AcceptXMR_DB \
    --mount type=bind,source=${PWD}/server/tests/testdata/cert,target=/server/tests/testdata/cert \
    --mount type=bind,source=${PWD}/acceptxmr.yaml,target=/acceptxmr.yaml \
    --env-file .env \
    acceptxmr
