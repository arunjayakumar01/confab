#!/usr/bin/env bash
# One-shot GCP setup for Confab against a real Google Workspace org. Idempotent.
#   scripts/gcp-setup.sh PROJECT_ID ACCOUNT_EMAIL
# Uses env overrides instead of `gcloud config set`, so your global gcloud config is untouched.
# Everything gcloud can do is done here; the two console pages it cannot do are printed at the end.
set -euo pipefail

PROJECT="${1:?usage: scripts/gcp-setup.sh PROJECT_ID ACCOUNT_EMAIL}"
ACCOUNT="${2:?usage: scripts/gcp-setup.sh PROJECT_ID ACCOUNT_EMAIL}"
export CLOUDSDK_CORE_ACCOUNT="$ACCOUNT" CLOUDSDK_CORE_PROJECT="$PROJECT"
KEY="$HOME/.config/prism/prism-relay-sa.json"
say() { printf '\n== %s\n' "$*"; }

say "gcloud session for $ACCOUNT"
gcloud auth print-access-token >/dev/null 2>&1 || gcloud auth login "$ACCOUNT"

say "project $PROJECT"
if ! gcloud projects describe "$PROJECT" >/dev/null 2>&1; then
  ORG="$(gcloud organizations list --format='value(ID)' | head -1)"
  [ -n "$ORG" ] || { echo "no organization visible for $ACCOUNT; an Internal consent screen needs one" >&2; exit 1; }
  gcloud projects create "$PROJECT" --organization="$ORG" --name="Prism"
  BA="$(gcloud billing accounts list --filter='open=true' --format='value(name)' | head -1 || true)"
  [ -z "$BA" ] || gcloud billing projects link "$PROJECT" --billing-account="${BA#billingAccounts/}"
fi

say "APIs"
gcloud services enable chat.googleapis.com workspaceevents.googleapis.com pubsub.googleapis.com iap.googleapis.com

say "OAuth consent screen (Internal), via the IAP brand API"
if [ -z "$(gcloud iap oauth-brands list --format='value(name)' 2>/dev/null)" ]; then
  gcloud iap oauth-brands create --application_title="Confab" --support_email="$ACCOUNT"
fi

say "Pub/Sub topic Google Chat publishes to, and the subscription the relay pulls"
# A freshly enabled Pub/Sub API answers "permission denied or may not exist" for a minute; retry.
for i in 1 2 3 4 5 6; do
  gcloud pubsub topics describe prism-events >/dev/null 2>&1 && break
  gcloud pubsub topics create prism-events && break
  echo "topic create refused (API still propagating?), retry $i/6 in 15s"; sleep 15
done
gcloud pubsub topics add-iam-policy-binding prism-events \
  --member=serviceAccount:chat-api-push@system.gserviceaccount.com --role=roles/pubsub.publisher >/dev/null
gcloud pubsub subscriptions describe prism-relay-pull >/dev/null 2>&1 \
  || gcloud pubsub subscriptions create prism-relay-pull --topic=prism-events --ack-deadline=30

say "service account the relay pulls with"
# Your own gcloud/ADC tokens cannot serve a background relay: Workspace orgs with Google Cloud
# session control reject their refresh with invalid_rapt. A service-account key has no such policy.
SA="prism-relay@$PROJECT.iam.gserviceaccount.com"
gcloud iam service-accounts describe "$SA" >/dev/null 2>&1 \
  || gcloud iam service-accounts create prism-relay --display-name="Prism relay"
gcloud pubsub subscriptions add-iam-policy-binding prism-relay-pull \
  --member="serviceAccount:$SA" --role=roles/pubsub.subscriber >/dev/null
if [ ! -f "$KEY" ]; then
  mkdir -p "$(dirname "$KEY")" && chmod 700 "$(dirname "$KEY")"
  gcloud iam service-accounts keys create "$KEY" --iam-account="$SA" || {
    echo "key creation refused (org policy iam.disableServiceAccountKeyCreation?): ask an org admin for an exception on $PROJECT" >&2
    exit 1
  }
fi

say ".env.real"
[ -f .env.real ] || sed -e "s/YOUR_PROJECT_ID/$PROJECT/g" .env.real.example > .env.real
grep -v '^GOOGLE_APPLICATION_CREDENTIALS=' .env.real > .env.real.tmp
printf 'GOOGLE_APPLICATION_CREDENTIALS=%s\n' "$KEY" >> .env.real.tmp && mv .env.real.tmp .env.real
echo "GOOGLE_APPLICATION_CREDENTIALS set; check PRISM_PUBSUB_TOPIC and PRISM_PUBSUB_SUBSCRIPTION name $PROJECT"

cat <<MSG

Done on GCP. Two console pages remain (no API for them), then paste the client id and secret into .env.real:

1. OAuth client, type Desktop app, name "Prism desktop":
   https://console.cloud.google.com/auth/clients/create?project=$PROJECT
   -> PRISM_CLIENT_ID / PRISM_CLIENT_SECRET in .env.real

2. Google Chat API configuration (needed for sending and for event subscriptions):
   https://console.cloud.google.com/apis/api/chat.googleapis.com/hangouts-chat?project=$PROJECT
   App name: Prism   Avatar URL: https://developers.google.com/workspace/chat/images/quickstart-app-avatar.png
   Description: Desktop client   Enable interactive features: off   -> Save

Then: make relay-real   (another terminal) make desktop-real
MSG
