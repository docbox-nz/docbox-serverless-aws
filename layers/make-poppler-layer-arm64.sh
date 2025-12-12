#!/usr/bin/env bash

# Build the container
docker buildx build --platform linux/arm64 -f poppler.Dockerfile -t poppler-lambda-layer-arm64.

# Run a container and copy out the zip then delete it
CONTAINER_ID=$(docker create --platform linux/arm64 poppler-lambda-layer-arm64)
docker cp $CONTAINER_ID:/poppler-lambda-layer.zip ./poppler-lambda-layer.zip
docker rm $CONTAINER_ID
