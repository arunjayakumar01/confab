# Troubleshooting Confab, the Google Chat desktop client

## Sending a message returns 403, but reading works

The Chat API isn't configured yet. Fill in the app name, avatar URL and description on the **Google Chat API, Configuration** page. See [Connect Your Workspace](../README.md#connect-your-google-workspace-account).

## The relay fails with `invalid_rapt`

Your organisation uses Google Cloud session control, which rejects gcloud's default credentials. Use the service account key that `scripts/gcp-setup.sh` creates, set through `GOOGLE_APPLICATION_CREDENTIALS` in `.env.real`.

## Messages from Google Chat don't appear

Check the relay log for `subscription created` after your first sign-in. If it isn't there, confirm that the Workspace Events and Pub/Sub APIs are enabled and that `PRISM_PUBSUB_TOPIC` in `.env.real` is correct.

## Calendar, Todos, Notes or huddles don't load

Each one needs its own API enabled in your project. See the table in [Connect Your Workspace](../README.md#connect-your-google-workspace-account). Adding a feature can also add a permission, so Google may ask you to consent again.

## The people picker says the directory isn't available

Your Workspace admin's Directory sharing setting decides whether the directory answers. People you already share a space with still appear.

## I can't see anyone typing or online, and read receipts are empty

These only work between people using Confab, because Google doesn't provide them to user sign-ins. If you are the only Confab user in your workspace, they stay empty.

## The Huddle button in a DM opens Google Chat

This is expected. No public Google API can ring a person, so Confab opens the conversation in Google Chat, where pressing Huddle rings them normally.

## macOS keeps asking for Keychain access

Unsigned development builds prompt each time they change. Set `PRISM_TOKEN_STORE=file` in `.env.real` to store tokens in a private file under your profile folder during development.

## Notifications appear to come from Terminal

A `cargo tauri dev` build isn't an app bundle, so macOS attributes its notifications to Terminal and asks permission once. A built app shows its own name.

## macOS asks to let the app control Google Chrome

Confab needs this to close the Chrome window when a huddle ends. If you decline, you can still close that window yourself.

## Google's consent screen shows the wrong app name

That name comes from the consent settings in your own Google Cloud project, so change it there.

## Still stuck?

[Open an issue](https://github.com/arunjayakumar01/confab/issues), and leave out tokens, client secrets and message contents.
