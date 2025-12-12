#!/usr/bin/env bash

# Build the container
docker buildx build --platform linux/amd64 -f poppler.Dockerfile -t poppler-lambda-layer-amd64 .

# Run a container and copy out the zip then delete it
CONTAINER_ID=$(docker create --platform linux/amd64 poppler-lambda-layer-amd64)
docker cp $CONTAINER_ID:/poppler-lambda-layer.zip ./poppler-lambda-layer.zip
docker rm $CONTAINER_ID
