#!/usr/bin/env bash
set -euo pipefail

IMAGE_REPO="${IMAGE_REPO:-desiders/ytdl_tg_bot.migration}"
IMAGE_TAG="${IMAGE_TAG:-dev-$(git rev-parse --short HEAD)}"
CACHE_REF="${CACHE_REF:-${IMAGE_REPO}:buildcache-migration-dev}"
PLATFORM="${PLATFORM:-linux/amd64}"
DOCKERFILE="${DOCKERFILE:-deployment/Dockerfile.migration.dev}"
CONTEXT_DIR="${CONTEXT_DIR:-.}"
BUILDER_NAME="${BUILDER_NAME:-ytdl-builder}"
NAMESPACE="${NAMESPACE:-dev}"
MIGRATION_COMMAND="${MIGRATION_COMMAND:-up}"

if ! command -v docker >/dev/null 2>&1; then
    echo "Docker not found" >&2
    exit 1
fi

if ! docker buildx version >/dev/null 2>&1; then
    echo "Docker buildx is required" >&2
    exit 1
fi

if ! docker buildx inspect "${BUILDER_NAME}" >/dev/null 2>&1; then
    echo "Creating buildx builder: ${BUILDER_NAME}"
    docker buildx create --name "${BUILDER_NAME}" --driver docker-container --bootstrap
fi

echo "Building and pushing ${IMAGE_REPO}:${IMAGE_TAG}"
echo "Using remote cache: ${CACHE_REF}"

docker buildx build \
    --builder "${BUILDER_NAME}" \
    --platform "${PLATFORM}" \
    --file "${DOCKERFILE}" \
    --tag "${IMAGE_REPO}:${IMAGE_TAG}" \
    --cache-from "type=registry,ref=${CACHE_REF}" \
    --cache-to "type=registry,ref=${CACHE_REF},mode=max" \
    --push \
    "${CONTEXT_DIR}"

echo "Pushed: ${IMAGE_REPO}:${IMAGE_TAG}"
echo "Next: IMAGE_REPO=${IMAGE_REPO} IMAGE_TAG=${IMAGE_TAG} PULL_POLICY=Always just k8s-migration ${NAMESPACE} ${MIGRATION_COMMAND}"
