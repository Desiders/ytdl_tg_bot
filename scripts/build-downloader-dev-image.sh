#!/usr/bin/env bash
set -euo pipefail

IMAGE_REPO="${IMAGE_REPO:-desiders/ytdl_tg_bot.downloader}"
IMAGE_TAG="${IMAGE_TAG:-dev-$(git rev-parse --short HEAD)}"
CACHE_REF="${CACHE_REF:-${IMAGE_REPO}:buildcache-downloader-dev}"
PLATFORM="${PLATFORM:-linux/amd64}"
DOCKERFILE="${DOCKERFILE:-deployment/Dockerfile.downloader.dev}"
CONTEXT_DIR="${CONTEXT_DIR:-.}"
BUILDER_NAME="${BUILDER_NAME:-ytdl-builder}"
NAMESPACE="${NAMESPACE:-dev}"

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
echo "Next: helm upgrade downloader ./charts/downloader -n ${NAMESPACE} --set downloader.image.repository=${IMAGE_REPO} --set downloader.image.tag=${IMAGE_TAG} --set downloader.image.pullPolicy=Always"
