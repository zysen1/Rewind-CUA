.PHONY: test demo eval

test:
	cargo test --locked --workspace

eval:
	cargo run --locked --quiet -- eval --out eval/results.json

demo:
	./demo/mistaken-delete.sh
