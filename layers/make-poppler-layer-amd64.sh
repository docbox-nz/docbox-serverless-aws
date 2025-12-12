#!/usr/bin/env bash
set -euo pipefail

# Build the container
docker buildx build --build-arg BASE_IMAGE=public.ecr.aws/lambda/provided:al2023 --platform linux/amd64 -f ./layers/poppler.Dockerfile -t poppler-lambda-layer-amd64 ./layers

# Run a container and copy out the zip then delete it
CONTAINER_ID=$(docker create --platform linux/amd64 poppler-lambda-layer-amd64)
docker cp $CONTAINER_ID:/tmp/poppler-lambda-layer.zip ./poppler-lambda-layer.zip
docker rm $CONTAINER_ID
