# Security policy

Please don't report security problems in public issues.

Use [private vulnerability reporting](https://github.com/arunjayakumar01/confab/security/advisories/new) instead, with the steps to reproduce the problem and the version or commit you tested. Leave out real tokens and message contents.

Confab is early software, so only the latest commit on `main` receives fixes.

## How Confab handles data

- Tokens are stored on your machine.
- The app sends your access token to the relay you run, which uses it to check your identity and space membership with Google.
- New message events pass through that relay, so treat it as part of your own infrastructure.
- There is no Confab-hosted server.
