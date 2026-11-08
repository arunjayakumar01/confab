# Contributing to Confab

Bug reports, ideas and pull requests are welcome.

## Set up

Follow [Run Confab locally](README.md#run-confab-locally). You don't need a Google account, because the project includes a local stand-in for Google.

## Before you open a pull request

```bash
make check
```

```bash
pnpm -C apps/desktop test
```

CI also runs the lint on Linux, which can catch problems a Mac doesn't.

## Good to know

- API tables, the relay protocol and app commands are in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Update them when you change a contract.
- Internal identifiers such as the bundle id, keychain service names and environment variable prefixes keep their values on purpose. Renaming them would sign people out and orphan their notes.
- Don't include tokens, client secrets, real message contents or screenshots of real accounts in issues, commits or pull requests.

## Report a security problem

See the [security policy](SECURITY.md).
