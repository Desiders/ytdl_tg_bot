# Migration

SeaORM migrations for the bot database. The crate is a workspace member, is built into the `desiders/ytdl_tg_bot.migration` image, and is a dev-dependency of `bot` so tests can apply the schema to a throwaway PostgreSQL container.

## In Kubernetes

```bash
just k8s-migration "${NAMESPACE}"        # up
just k8s-migration "${NAMESPACE}" down   # any migrator command
```

Set `IMAGE_REPO` and `IMAGE_TAG` together to run a dev image (`just docker-push-dev-migration` prints the exact command).

## Migrator CLI

All commands read `DATABASE_URL`, for example `postgres://user:password@127.0.0.1:5432/api`. Use `just k8s-port-forward-db "${NAMESPACE}"` to reach the cluster database locally.

- Generate a new migration file
    ```sh
    cargo run -- generate MIGRATION_NAME
    ```
- Apply all pending migrations
    ```sh
    cargo run
    ```
    ```sh
    cargo run -- up
    ```
- Apply first 10 pending migrations
    ```sh
    cargo run -- up -n 10
    ```
- Rollback last applied migrations
    ```sh
    cargo run -- down
    ```
- Rollback last 10 applied migrations
    ```sh
    cargo run -- down -n 10
    ```
- Drop all tables from the database, then reapply all migrations
    ```sh
    cargo run -- fresh
    ```
- Rollback all applied migrations, then reapply all migrations
    ```sh
    cargo run -- refresh
    ```
- Rollback all applied migrations
    ```sh
    cargo run -- reset
    ```
- Check the status of all migrations
    ```sh
    cargo run -- status
    ```

## Regenerate Entities

After adding a migration, regenerate `bot/src/database/models` from the migrated database:

```sh
just generate-entities-from-db "${NAMESPACE}"
```

That recipe port-forwards nothing itself; run `just k8s-port-forward-db "${NAMESPACE}"` first. It calls `sea-orm-cli generate entity -o ../bot/src/database/models --date-time-crate time --with-prelude none --banner-version patch --entity-format dense` and removes the generated `mod.rs`, since `bot/src/database/models.rs` declares the modules.
