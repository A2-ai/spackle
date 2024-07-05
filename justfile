setup:
    lefthook install
    cd frontend && bun install

run *args="":
    cargo run -p spackle-cli {{args}}

test:
    cargo test --workspace

install:
    cargo install --path=cli