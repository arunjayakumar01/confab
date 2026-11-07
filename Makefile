.PHONY: twin relay web desktop desktop-b up down check build-desktop floci e2e gcp-setup relay-real desktop-real

twin:
	cargo run -p google-twin
relay:
	cargo run -p relay
web:
	pnpm -C apps/desktop dev
desktop:
	cd apps/desktop && cargo tauri dev
# Two `tauri dev` processes clash on the Vite port, so profile B reuses the binary built by `make desktop`.
desktop-b:
	PRISM_PROFILE=b target/debug/prism-desktop
up:
	docker compose up --build
down:
	docker compose down
check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace
	pnpm -C apps/desktop typecheck
build-desktop:
	cd apps/desktop && cargo tauri build
floci:
	floci gcp start && floci gcp wait && node tests/floci-setup.mjs
e2e:
	node tests/e2e-typing.mjs
# Real Google Workspace: `make gcp-setup PROJECT=my-project ACCOUNT=you@your-org` once, then the two console pages it prints.
gcp-setup:
	scripts/gcp-setup.sh $(PROJECT) $(ACCOUNT)
relay-real:
	set -a; . ./.env.real; set +a; cargo run -p relay
desktop-real:
	set -a; . ./.env.real; set +a; cd apps/desktop && cargo tauri dev
