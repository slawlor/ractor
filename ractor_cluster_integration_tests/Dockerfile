# Build from root of repository with
#
# docker build -f ractor_cluster_integration_tests/Dockerfile --build-arg FEATURES="list,of,features" . -t ractor_integration_tests:latest
# docker run -it ractor_integration_tests:latest help

FROM rust:latest
WORKDIR /usr/src/app
RUN apt-get update && apt-get install -y cmake build-essential
COPY . .
ARG FEATURES
RUN cargo install --path ractor_cluster_integration_tests/ -F $FEATURES
RUN mv /usr/local/cargo/bin/ractor_cluster_integration_tests /usr/local/bin/ractor_cluster_integration_tests
RUN mv /usr/src/app/ractor_cluster_integration_tests/test-ca/ /usr/local/bin/test-ca
WORKDIR /usr/local/bin

ENTRYPOINT [ "ractor_cluster_integration_tests" ]
# ENTRYPOINT ["/bin/bash"]
