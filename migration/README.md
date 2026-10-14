# Migration

SeaORM migrations for the bot database. The crate is a workspace member, is built into the `ghcr.io/desiders/ytdl_tg_bot.migration` image, and is a dev-dependency of `bot` so tests can apply the schema to a throwaway PostgreSQL container.

Run commands from the repository root. For Docker Compose startup and migrations, see
[Local Development](../README.md#local-development).

## In Kubernetes

```bash
just k8s-migration "${NAMESPACE}" up
just k8s-migration "${NAMESPACE}" down   # any migrator command
```

## Migrator CLI

Generate a migration from the crate directory so the file is created in `migration/src/`:

```bash
(cd migration && cargo run --locked -p migration -- generate MIGRATION_NAME)
```

The generator registers it in `migration/src/lib.rs`. Implement its `up` and `down` methods
before applying it. Generation does not require a database connection.

Database commands read `DATABASE_URL`, for example
`postgres://<db-user>:<db-password>@127.0.0.1:<db-port>/<db-name>`. Use the credentials and
published port described in [Local Development](../README.md#regenerate-database-models),
or port-forward the cluster database with `just k8s-port-forward-db "${NAMESPACE}"`.

| Command | Effect |
| --- | --- |
| `cargo run --locked -p migration -- up` | Apply all pending migrations |
| `cargo run --locked -p migration -- up -n 10` | Apply the first 10 pending migrations |
| `cargo run --locked -p migration -- down` | Roll back the last applied migration |
| `cargo run --locked -p migration -- down -n 10` | Roll back the last 10 applied migrations |
| `cargo run --locked -p migration -- fresh` | Drop all tables, then reapply all migrations |
| `cargo run --locked -p migration -- refresh` | Roll back all applied migrations, then reapply them |
| `cargo run --locked -p migration -- reset` | Roll back all applied migrations |
| `cargo run --locked -p migration -- status` | Check migration status |

## Regenerate Entities

After applying migrations, follow [Regenerate Database Models](../README.md#regenerate-database-models).
For a Kubernetes database, use the connection instructions in [Operations](../README.md#operations).
