.PHONY: db-migrate db-purify db-load db-reload db-clear-structs

db-migrate db-purify db-load db-reload db-clear-structs:
	cargo run --manifest-path "$(dir $(abspath $(lastword $(MAKEFILE_LIST))))Cargo.toml" --bin db -- $@
