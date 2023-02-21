# Build from root of repository with
#
# docker build -f ractor_cluster_integration_tests/Dockerfile . -t ractor_integration_tests:latest
# docker run -it ractor_integration_tests:latest help

FROM rust:latest
WORKDIR /usr/src/app
COPY . .
RUN cargo install --path ractor_cluster_integration_tests/
RUN mv /usr/local/cargo/bin/ractor_cluster_integration_tests /usr/local/bin/ractor_cluster_integration_tests
RUN mv /usr/src/app/ractor_cluster_integration_tests/test-ca/ /usr/local/bin/test-ca
WORKDIR /usr/local/bin

ENTRYPOINT [ "ractor_cluster_integration_tests" ]
# ENTRYPOINT ["/bin/bash"]
