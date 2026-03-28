# Kubernetes Migration Plan

This file is an implementation roadmap for moving the project toward Kubernetes, starting with `k3s`.

It is intentionally separate from [AGENTS.md](/workspace/AGENTS.md), which describes current architecture and rules rather than future rollout steps.

## Current Decision

- Use `k3s` for learning and initial Kubernetes deployment work.
- Start by migrating only the `downloader` service to Kubernetes.
- Keep the `bot` outside Kubernetes at first.
- Keep `NodeRouter` logic in the bot. Kubernetes does not replace cookie-aware or capacity-aware node selection.

## Why Start With Downloader

- It is operationally simpler than the bot.
- It has a clear network boundary: gRPC in, media stream out.
- It is the right place to learn K8s basics first:
  - `Deployment`
  - `Service`
  - `Secret`
  - `ConfigMap`
  - `PersistentVolumeClaim`
- It lets the bot continue working while Kubernetes is introduced gradually.

## Non-Goals For The First K8s Phase

- Do not migrate the bot in the first phase.
- Do not replace `NodeRouter` with Kubernetes Service balancing.
- Do not introduce Argo CD yet.
- Do not redesign the downloader protocol.
- Do not try to solve every operational concern in the first pass.

## Target End State For Phase 1

- A `k3s` cluster runs one or more downloader pods.
- The bot can call the downloader through a stable address exposed by Kubernetes.
- Downloader config, auth token, and TLS material are managed through K8s resources.
- Cookies and other mutable downloader data are mounted explicitly.
- Manual image rollout is acceptable.

## Phase Plan

## Phase 0: Prepare And Stabilize

- Keep Docker/Compose deployment working as the reference environment.
- Make sure downloader image builds cleanly and predictably.
- Keep config paths and TLS paths explicit.
- Avoid mixing Kubernetes-specific assumptions into bot logic.

Success criteria:
- Local Docker deployment remains the source of truth.
- Downloader image can be built and pushed independently.

## Phase 1: Downloader-Only k3s Prototype

- Install `k3s` on one server.
- Deploy only the downloader there.
- Create Kubernetes resources for:
  - `Namespace`
  - `Deployment`
  - `Service`
  - `Secret` for auth tokens
  - `Secret` for TLS cert/key if TLS is enabled
  - `ConfigMap` or mounted config file for downloader config
- Point the bot to that downloader node as an external node.

Questions to settle:
- Whether the bot connects through cluster-internal networking, public IP, or a tunneled/private address.
- Whether the first prototype uses plaintext gRPC or TLS.

Success criteria:
- Bot can fetch media info and download media through the Kubernetes-hosted downloader.
- Existing local/non-K8s bot deployment still works.

## Phase 2: Stateful Downloader Inputs

- Decide how cookies are delivered to downloader pods.
- Decide how yt-dlp plugins are delivered to downloader pods.
- Decide whether downloader data should be immutable in the image or mounted separately.

Likely Kubernetes resources:
- `Secret` or mounted volume for cookies
- `ConfigMap` or image-baked plugin directory for plugins
- `PersistentVolumeClaim` only if mutable runtime data is truly needed

Success criteria:
- Cookie-aware routing still works from the bot’s perspective.
- Recreating a pod does not silently break downloader capability expectations.

## Phase 3: TLS And Node Identity In Kubernetes

- Decide whether downloader pods will expose TLS directly or behind another layer.
- If downloader serves TLS directly:
  - mount cert/key into pod
  - configure downloader `[tls]`
  - configure bot `[download.tls]`
- If the bot addresses the downloader by IP, cert SAN must match that IP.
- If the bot addresses the downloader by DNS name, cert SAN must match that name.

Success criteria:
- Bot successfully connects to K8s downloader over the chosen transport.
- TLS setup does not require bot-side routing logic changes.

## Phase 4: Operational Rollout Model

- Keep image-based updates as the primary model.
- Publish versioned downloader images.
- Roll out new images manually first.
- Delay automatic image update machinery until after the cluster workflow is understood.

Not first-phase tasks:
- automatic image promotion
- Argo CD
- image update controllers

Success criteria:
- Updating downloader image in Kubernetes is a controlled, repeatable manual process.
- Rollback path is documented and tested.

## Phase 5: Optional Bot Migration

- Only after downloader-on-K8s is stable, evaluate moving the bot too.
- If the bot moves:
  - model database connectivity explicitly
  - model Telegram API connectivity explicitly
  - move bot config and secrets into Kubernetes
- Keep handler/interactor interfaces stable while changing deployment shape.

Success criteria:
- Bot migration is optional and independent from downloader success.

## Required K8s Resources For Downloader First

Minimum expected set:
- `Namespace`
- `Deployment`
- `Service`
- `Secret` for auth token
- `Secret` for TLS materials if TLS is enabled
- `ConfigMap` or mounted config file

Possible later additions:
- `Ingress` or `Gateway`
- `PersistentVolumeClaim`
- `HorizontalPodAutoscaler`
- `NetworkPolicy`

## Risks

- Treating Kubernetes like a replacement for `NodeRouter` would be a design mistake.
- Moving bot and downloader at the same time would make debugging much harder.
- TLS, cookies, and mutable yt-dlp-related state are the main operational sharp edges.
- Auto-updating images too early will make failures harder to diagnose.

## Open Decisions

- Downloader address model in Kubernetes:
  - cluster-internal only
  - externally reachable service
  - private tunnel into cluster
- TLS in phase 1:
  - start plaintext for first prototype
  - or require TLS immediately
- Cookie delivery:
  - secret
  - mounted file
  - another management flow
- Image update workflow after manual phase:
  - custom updater
  - Kubernetes-native image automation
  - GitOps later

## Recommended First Practical Step

1. Bring up a single-node `k3s` cluster.
2. Deploy only one downloader instance there.
3. Expose it with a simple `Service`.
4. Point the existing bot to it as one external node.
5. Validate media info and download flow before adding more infrastructure.

## Progress Checklist

- [ ] Single-node `k3s` cluster installed
- [ ] Downloader image pushed to a registry reachable by the cluster
- [ ] Downloader `Deployment` created
- [ ] Downloader `Service` created
- [ ] Downloader auth token provided through `Secret`
- [ ] Downloader config provided through `ConfigMap` or mounted file
- [ ] Bot connected to K8s downloader successfully
- [ ] TLS decision implemented
- [ ] Cookies/plugins strategy implemented
- [ ] Manual image rollout procedure documented
- [ ] Decision made on whether to migrate the bot
