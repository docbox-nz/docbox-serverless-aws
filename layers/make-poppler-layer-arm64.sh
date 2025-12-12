#!/usr/bin/env bash
set -euo pipefail

# Build the container
docker buildx build --build-arg BASE_IMAGE=public.ecr.aws/lambda/provided:al2023-arm64 --platform linux/arm64 -f ./layers/poppler.Dockerfile -t poppler-lambda-layer-arm64 --load ./layers

# Run a container and copy out the zip then delete it
CONTAINER_ID=$(docker create --platform linux/arm64 poppler-lambda-layer-arm64)
docker cp $CONTAINER_ID:/tmp/poppler-lambda-layer.zip ./poppler-lambda-layer.zip
docker rm $CONTAINER_ID
