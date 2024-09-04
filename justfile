setup:
    lefthook install

run *args="":
    cargo run -p spackle-cli {{args}}

test:
    cargo test --workspace

install:
    cargo install --path=cli